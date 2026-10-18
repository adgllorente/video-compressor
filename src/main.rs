#![cfg_attr(windows, windows_subsystem = "windows")]

mod encoder;
mod ffmpeg;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use encoder::{Codec, EncodeError, Job, JobHandle, Outcome, Probe, Resolution, Settings, Speed};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "avi", "webm", "wmv", "flv", "mts", "m2ts", "ts", "3gp", "mpg", "mpeg", "vob",
];
const SIZE_PRESETS: &[f64] = &[8.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Compresor de vídeo")
            .with_inner_size([820.0, 600.0])
            .with_min_inner_size([620.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native("Compresor de vídeo", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

enum FfmpegState {
    Searching,
    Downloading(f32),
    Ready(PathBuf),
    Error(String),
}

enum ProbeState {
    Unknown,
    Running,
    Ok(Probe),
    Err(String),
}

enum Status {
    Pending,
    Working(f32),
    Done(Outcome),
    Skipped(String),
    Failed(String),
    Cancelled,
}

struct Item {
    id: u64,
    path: PathBuf,
    size: u64,
    probe: ProbeState,
    status: Status,
}

enum Msg {
    FfmpegFound(Option<PathBuf>),
    FfmpegDownload(f32),
    FfmpegDownloaded(Result<PathBuf, String>),
    Probed(u64, Result<Probe, String>),
    Progress(u64, f32),
    Finished(u64, Status),
    BatchDone,
}

struct App {
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ffmpeg: FfmpegState,
    items: Vec<Item>,
    next_id: u64,
    settings: Settings,
    running: Option<JobHandle>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let app = App {
            ctx: ctx.clone(),
            tx,
            rx,
            ffmpeg: FfmpegState::Searching,
            items: Vec::new(),
            next_id: 0,
            settings: Settings {
                target_mb: 25.0,
                codec: Codec::H264,
                speed: Speed::Balanced,
                resolution: Resolution::Auto,
                output_dir: None,
            },
            running: None,
        };
        app.spawn(|tx| {
            let _ = tx.send(Msg::FfmpegFound(ffmpeg::locate()));
        });
        app
    }

    fn spawn(&self, work: impl FnOnce(Sender<Msg>) + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            work(tx);
            ctx.request_repaint();
        });
    }

    fn sender(&self) -> impl Fn(Msg) + Send + Clone + 'static {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        move |msg| {
            let _ = tx.send(msg);
            ctx.request_repaint();
        }
    }

    fn ffmpeg_path(&self) -> Option<&Path> {
        match &self.ffmpeg {
            FfmpegState::Ready(p) => Some(p),
            _ => None,
        }
    }

    fn start_download(&mut self) {
        self.ffmpeg = FfmpegState::Downloading(0.0);
        let send = self.sender();
        std::thread::spawn(move || {
            let result = ffmpeg::download(|p| send(Msg::FfmpegDownload(p)));
            send(Msg::FfmpegDownloaded(result));
        });
    }

    fn handle_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::FfmpegFound(Some(path)) | Msg::FfmpegDownloaded(Ok(path)) => {
                    self.ffmpeg = FfmpegState::Ready(path)
                }
                Msg::FfmpegFound(None) => self.start_download(),
                Msg::FfmpegDownload(p) => self.ffmpeg = FfmpegState::Downloading(p),
                Msg::FfmpegDownloaded(Err(e)) => self.ffmpeg = FfmpegState::Error(e),
                Msg::Probed(id, result) => {
                    if let Some(item) = self.item_mut(id) {
                        item.probe = match result {
                            Ok(p) => ProbeState::Ok(p),
                            Err(e) => ProbeState::Err(e),
                        };
                    }
                }
                Msg::Progress(id, p) => {
                    if let Some(item) = self.item_mut(id) {
                        item.status = Status::Working(p);
                    }
                }
                Msg::Finished(id, status) => {
                    if let Some(item) = self.item_mut(id) {
                        item.status = status;
                    }
                }
                Msg::BatchDone => self.running = None,
            }
        }
    }

    fn item_mut(&mut self, id: u64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if path.is_dir() || self.items.iter().any(|i| i.path == path) {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            self.items.push(Item { id: self.next_id, path, size, probe: ProbeState::Unknown, status: Status::Pending });
            self.next_id += 1;
        }
    }

    fn probe_pending(&mut self) {
        let Some(ffmpeg) = self.ffmpeg_path().map(Path::to_path_buf) else { return };
        let send = self.sender();
        for item in self.items.iter_mut().filter(|i| matches!(i.probe, ProbeState::Unknown)) {
            item.probe = ProbeState::Running;
            let (id, path, ffmpeg, send) = (item.id, item.path.clone(), ffmpeg.clone(), send.clone());
            std::thread::spawn(move || send(Msg::Probed(id, encoder::probe(&ffmpeg, &path))));
        }
    }

    fn start_batch(&mut self) {
        let Some(ffmpeg) = self.ffmpeg_path().map(Path::to_path_buf) else { return };
        let target_bytes = (self.settings.target_mb * 1024.0 * 1024.0) as u64;
        let mut queue = Vec::new();
        for item in &mut self.items {
            if matches!(item.status, Status::Done(_)) {
                continue;
            }
            if let ProbeState::Err(e) = &item.probe {
                item.status = Status::Failed(e.clone());
                continue;
            }
            if item.size > 0 && item.size <= target_bytes {
                item.status = Status::Skipped("Ya ocupa menos que el tamaño final".into());
                continue;
            }
            item.status = Status::Working(0.0);
            queue.push((item.id, item.path.clone()));
        }
        if queue.is_empty() {
            return;
        }

        let job = Job::new();
        self.running = Some(job.handle());
        let settings = self.settings.clone();
        let send = self.sender();
        std::thread::spawn(move || {
            for (id, path) in queue {
                if job.is_cancelled() {
                    send(Msg::Finished(id, Status::Cancelled));
                    continue;
                }
                let status = match encoder::probe(&ffmpeg, &path) {
                    Err(e) => Status::Failed(e),
                    Ok(probe) => {
                        let progress_send = send.clone();
                        match job.encode(&ffmpeg, &path, probe, &settings, &mut |p| {
                            progress_send(Msg::Progress(id, p))
                        }) {
                            Ok(outcome) => Status::Done(outcome),
                            Err(EncodeError::Cancelled) => Status::Cancelled,
                            Err(EncodeError::Failed(e)) => Status::Failed(e),
                        }
                    }
                };
                send(Msg::Finished(id, status));
            }
            send(Msg::BatchDone);
        });
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .set_title("Selecciona vídeos")
            .add_filter("Vídeos", VIDEO_EXTENSIONS)
            .add_filter("Todos los archivos", &["*"])
            .pick_files()
        {
            self.add_files(files);
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Compresor de vídeo");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &self.ffmpeg {
                FfmpegState::Searching => {
                    ui.spinner();
                    ui.label("Buscando ffmpeg…");
                }
                FfmpegState::Downloading(p) => {
                    ui.add(egui::ProgressBar::new(*p).desired_width(140.0).show_percentage());
                    ui.label("Descargando ffmpeg (solo la primera vez)");
                }
                FfmpegState::Ready(_) => {
                    ui.weak("ffmpeg listo");
                }
                FfmpegState::Error(e) => {
                    let e = e.clone();
                    if ui.button("Reintentar").clicked() {
                        self.start_download();
                    }
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let idle = self.running.is_none();
            if ui.add_enabled(idle, egui::Button::new("➕ Añadir vídeos…")).clicked() {
                self.pick_files();
            }
            if ui.add_enabled(idle && !self.items.is_empty(), egui::Button::new("Vaciar lista")).clicked() {
                self.items.clear();
            }
            if idle && self.items.iter().any(|i| matches!(i.status, Status::Done(_)))
                && ui.button("Quitar terminados").clicked() {
                    self.items.retain(|i| !matches!(i.status, Status::Done(_)));
                }
        });
        ui.add_space(4.0);
    }

    fn file_list(&mut self, ui: &mut egui::Ui) {
        if self.items.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Arrastra aquí tus vídeos\no pulsa «Añadir vídeos…»").size(18.0).weak());
            });
            return;
        }
        let idle = self.running.is_none();
        let target_bytes = (self.settings.target_mb * 1024.0 * 1024.0) as u64;
        let mut remove = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for item in &self.items {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let name = item.path.file_name().unwrap_or_default().to_string_lossy();
                        ui.strong(name).on_hover_text(item.path.display().to_string());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if idle && ui.small_button("✖").on_hover_text("Quitar de la lista").clicked() {
                                remove = Some(item.id);
                            }
                            let mut info = encoder::format_bytes(item.size);
                            match &item.probe {
                                ProbeState::Ok(p) => info = format!("{} · {info}", encoder::format_duration(p.duration)),
                                ProbeState::Running => {
                                    ui.spinner();
                                }
                                _ => {}
                            }
                            ui.weak(info);
                        });
                    });
                    match &item.status {
                        Status::Pending => {
                            let hint = match &item.probe {
                                ProbeState::Err(e) => Some((e.clone(), true)),
                                _ if item.size > 0 && item.size <= target_bytes => {
                                    Some(("Ya ocupa menos que el tamaño final; se omitirá".into(), false))
                                }
                                _ => None,
                            };
                            match hint {
                                Some((text, true)) => {
                                    ui.colored_label(ui.visuals().error_fg_color, text);
                                }
                                Some((text, false)) => {
                                    ui.weak(text);
                                }
                                None => {
                                    ui.weak("En cola");
                                }
                            }
                        }
                        Status::Working(p) => {
                            ui.add(egui::ProgressBar::new(*p).show_percentage().animate(true));
                        }
                        Status::Done(outcome) => {
                            ui.horizontal(|ui| {
                                let saved = 100.0 - outcome.size as f64 / item.size.max(1) as f64 * 100.0;
                                ui.colored_label(
                                    egui::Color32::from_rgb(60, 170, 90),
                                    format!(
                                        "✔ {} (−{saved:.0}%) → {}",
                                        encoder::format_bytes(outcome.size),
                                        outcome.output.file_name().unwrap_or_default().to_string_lossy()
                                    ),
                                );
                                if ui.small_button("Mostrar").clicked() {
                                    reveal(&outcome.output);
                                }
                            });
                        }
                        Status::Skipped(why) => {
                            ui.weak(why);
                        }
                        Status::Failed(e) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        Status::Cancelled => {
                            ui.weak("Cancelado");
                        }
                    }
                });
                ui.add_space(2.0);
            }
        });
        if let Some(id) = remove {
            self.items.retain(|i| i.id != id);
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let idle = self.running.is_none();
        ui.add_enabled_ui(idle, |ui| {
            egui::Grid::new("ajustes").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Tamaño final");
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.target_mb)
                            .range(1.0..=100_000.0)
                            .speed(1.0)
                            .max_decimals(1)
                            .suffix(" MB"),
                    );
                    for &mb in SIZE_PRESETS {
                        if ui.selectable_label(self.settings.target_mb == mb, format!("{mb:.0}")).clicked() {
                            self.settings.target_mb = mb;
                        }
                    }
                });
                ui.end_row();

                ui.label("Códec");
                egui::ComboBox::from_id_salt("codec").selected_text(self.settings.codec.label()).show_ui(ui, |ui| {
                    for c in [Codec::H264, Codec::H265] {
                        ui.selectable_value(&mut self.settings.codec, c, c.label());
                    }
                });
                ui.end_row();

                ui.label("Velocidad");
                ui.horizontal(|ui| {
                    for s in [Speed::Fast, Speed::Balanced, Speed::Slow] {
                        ui.selectable_value(&mut self.settings.speed, s, s.label());
                    }
                });
                ui.end_row();

                ui.label("Resolución");
                egui::ComboBox::from_id_salt("res").selected_text(self.settings.resolution.label()).show_ui(ui, |ui| {
                    for r in Resolution::ALL {
                        ui.selectable_value(&mut self.settings.resolution, r, r.label());
                    }
                });
                ui.end_row();

                ui.label("Guardar en");
                ui.horizontal(|ui| {
                    let text = match &self.settings.output_dir {
                        Some(d) => d.display().to_string(),
                        None => "La misma carpeta que cada vídeo".into(),
                    };
                    ui.label(text);
                    if ui.button("Cambiar…").clicked()
                        && let Some(dir) = rfd::FileDialog::new().set_title("Carpeta de destino").pick_folder()
                    {
                        self.settings.output_dir = Some(dir);
                    }
                    if self.settings.output_dir.is_some() && ui.small_button("✖").clicked() {
                        self.settings.output_dir = None;
                    }
                });
                ui.end_row();
            });
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if let Some(handle) = &self.running {
                if ui.add(egui::Button::new(egui::RichText::new("Cancelar").size(16.0))).clicked() {
                    handle.cancel();
                }
                ui.spinner();
                ui.label("Comprimiendo… (dos pasadas por vídeo)");
            } else {
                let pending = self.items.iter().filter(|i| !matches!(i.status, Status::Done(_))).count();
                let ready = self.ffmpeg_path().is_some() && pending > 0;
                let label = egui::RichText::new(format!("Comprimir {pending} vídeo(s)")).size(16.0).strong();
                if ui.add_enabled(ready, egui::Button::new(label).min_size(egui::vec2(180.0, 32.0))).clicked() {
                    self.start_batch();
                }
            }
        });
        ui.add_space(8.0);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_messages();

        let (dropped, hovering) = ui.ctx().input(|i| {
            (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>(), !i.raw.hovered_files.is_empty())
        });
        if self.running.is_none() && !dropped.is_empty() {
            self.add_files(dropped);
        }
        self.probe_pending();

        egui::Panel::top("cabecera").show(ui, |ui| self.header(ui));
        egui::Panel::bottom("ajustes").show(ui, |ui| self.controls(ui));
        egui::CentralPanel::default().show(ui, |ui| self.file_list(ui));

        if hovering {
            let rect = ui.ctx().content_rect();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("soltar")));
            painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(170));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Suelta para añadir",
                egui::FontId::proportional(26.0),
                egui::Color32::WHITE,
            );
        }
    }
}

fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}
