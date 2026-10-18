use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MIB: f64 = 1024.0 * 1024.0;
const CONTAINER_OVERHEAD: f64 = 0.96;
const MIN_VIDEO_KBPS: f64 = 40.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264 (máxima compatibilidad)",
            Codec::H265 => "H.265 (mejor calidad, menos compatible)",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Fast,
    Balanced,
    Slow,
}

impl Speed {
    pub fn label(self) -> &'static str {
        match self {
            Speed::Fast => "Rápida",
            Speed::Balanced => "Equilibrada",
            Speed::Slow => "Lenta (mejor calidad)",
        }
    }

    fn preset(self) -> &'static str {
        match self {
            Speed::Fast => "veryfast",
            Speed::Balanced => "medium",
            Speed::Slow => "slow",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Auto,
    Original,
    Max(u32),
}

impl Resolution {
    pub const ALL: [Resolution; 6] = [
        Resolution::Auto,
        Resolution::Original,
        Resolution::Max(1080),
        Resolution::Max(720),
        Resolution::Max(480),
        Resolution::Max(360),
    ];

    pub fn label(self) -> String {
        match self {
            Resolution::Auto => "Automática según tamaño".into(),
            Resolution::Original => "Original".into(),
            Resolution::Max(h) => format!("Máximo {h}p"),
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub target_mb: f64,
    pub codec: Codec,
    pub speed: Speed,
    pub resolution: Resolution,
    pub output_dir: Option<PathBuf>,
}

#[derive(Clone, Copy)]
pub struct Probe {
    pub duration: f64,
    pub has_audio: bool,
}

pub struct Outcome {
    pub output: PathBuf,
    pub size: u64,
}

pub enum EncodeError {
    Cancelled,
    Failed(String),
}

impl From<String> for EncodeError {
    fn from(msg: String) -> Self {
        EncodeError::Failed(msg)
    }
}

pub fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn probe(ffmpeg: &Path, input: &Path) -> Result<Probe, String> {
    let out = command(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("No se pudo ejecutar ffmpeg: {e}"))?;
    let info = String::from_utf8_lossy(&out.stderr);

    let duration = info
        .lines()
        .find_map(|l| l.trim().strip_prefix("Duration: "))
        .and_then(|rest| rest.split(',').next())
        .and_then(parse_timestamp)
        .filter(|d| *d > 0.0)
        .ok_or_else(|| "No se pudo leer la duración (¿es un vídeo válido?)".to_string())?;
    let has_video = info.lines().any(|l| l.contains("Stream #") && l.contains("Video:"));
    if !has_video {
        return Err("El archivo no contiene vídeo".into());
    }
    let has_audio = info.lines().any(|l| l.contains("Stream #") && l.contains("Audio:"));
    Ok(Probe { duration, has_audio })
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

struct Plan {
    video_kbps: f64,
    audio_kbps: Option<u32>,
    max_height: Option<u32>,
}

fn plan(settings: &Settings, probe: Probe) -> Result<Plan, String> {
    let total_kbps = settings.target_mb * MIB * 8.0 / probe.duration / 1000.0 * CONTAINER_OVERHEAD;
    let audio_kbps = probe.has_audio.then_some(match total_kbps {
        t if t >= 1500.0 => 128,
        t if t >= 600.0 => 96,
        t if t >= 250.0 => 64,
        _ => 32,
    });
    let video_kbps = total_kbps - audio_kbps.unwrap_or(0) as f64;
    if video_kbps < MIN_VIDEO_KBPS {
        let min_mb = (MIN_VIDEO_KBPS + audio_kbps.unwrap_or(0) as f64) * 1000.0 * probe.duration
            / 8.0
            / MIB
            / CONTAINER_OVERHEAD;
        return Err(format!(
            "Tamaño demasiado pequeño para {}; mínimo razonable ≈ {:.0} MB",
            format_duration(probe.duration),
            min_mb.ceil()
        ));
    }
    let max_height = match settings.resolution {
        Resolution::Original => None,
        Resolution::Max(h) => Some(h),
        Resolution::Auto => match video_kbps {
            v if v < 400.0 => Some(360),
            v if v < 900.0 => Some(480),
            v if v < 2000.0 => Some(720),
            v if v < 4500.0 => Some(1080),
            _ => None,
        },
    };
    Ok(Plan { video_kbps, audio_kbps, max_height })
}

fn output_path(input: &Path, settings: &Settings) -> PathBuf {
    let dir = settings
        .output_dir
        .clone()
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let base = format!("{stem}_{}MB", format_mb(settings.target_mb));
    let mut candidate = dir.join(format!("{base}.mp4"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} ({n}).mp4"));
        n += 1;
    }
    candidate
}

pub struct Job {
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Job {
    pub fn new() -> Self {
        Job { cancel: Arc::new(AtomicBool::new(false)), child: Arc::new(Mutex::new(None)) }
    }

    pub fn handle(&self) -> JobHandle {
        JobHandle { cancel: self.cancel.clone(), child: self.child.clone() }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn encode(
        &self,
        ffmpeg: &Path,
        input: &Path,
        probe: Probe,
        settings: &Settings,
        progress: &mut dyn FnMut(f32),
    ) -> Result<Outcome, EncodeError> {
        let mut plan = plan(settings, probe)?;
        let output = output_path(input, settings);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let work_dir = std::env::temp_dir().join(format!("compresor-video-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&work_dir).map_err(|e| format!("No se pudo crear carpeta temporal: {e}"))?;

        let pass = |plan: &Plan, n: u8, from: f32, progress: &mut dyn FnMut(f32)| {
            self.run_pass(ffmpeg, input, &output, &work_dir, settings, plan, n, probe.duration, &mut |p| {
                progress(from + p * 0.5)
            })
        };
        let result = (|| {
            pass(&plan, 1, 0.0, progress)?;
            pass(&plan, 2, 0.5, progress)?;

            let target_bytes = settings.target_mb * MIB;
            let size = file_size(&output)?;
            if size as f64 > target_bytes {
                plan.video_kbps *= target_bytes / size as f64 * 0.97;
                pass(&plan, 2, 0.5, progress)?;
            }
            Ok(Outcome { size: file_size(&output)?, output: output.clone() })
        })();

        let _ = std::fs::remove_dir_all(&work_dir);
        if result.is_err() {
            let _ = std::fs::remove_file(&output);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn run_pass(
        &self,
        ffmpeg: &Path,
        input: &Path,
        output: &Path,
        work_dir: &Path,
        settings: &Settings,
        plan: &Plan,
        pass: u8,
        duration: f64,
        progress: &mut dyn FnMut(f32),
    ) -> Result<(), EncodeError> {
        if self.is_cancelled() {
            return Err(EncodeError::Cancelled);
        }

        let mut cmd = command(ffmpeg);
        cmd.current_dir(work_dir)
            .args(["-y", "-hide_banner", "-nostats", "-loglevel", "error", "-progress", "pipe:1", "-i"])
            .arg(input)
            .args(["-map", "0:v:0"]);
        if let Some(h) = plan.max_height {
            cmd.arg("-vf").arg(format!(
                "scale='trunc(iw*min(1,{h}/min(iw,ih))/2)*2':'trunc(ih*min(1,{h}/min(iw,ih))/2)*2'"
            ));
        }
        let bitrate = format!("{}k", plan.video_kbps.round() as u64);
        cmd.args(["-b:v", &bitrate, "-preset", settings.speed.preset(), "-pix_fmt", "yuv420p"]);
        match settings.codec {
            Codec::H264 => {
                cmd.args(["-c:v", "libx264", "-pass", &pass.to_string(), "-passlogfile", "pass"]);
            }
            Codec::H265 => {
                cmd.args(["-c:v", "libx265", "-tag:v", "hvc1", "-x265-params"])
                    .arg(format!("pass={pass}:stats=x265.log:log-level=error"));
            }
        }
        if pass == 1 {
            cmd.args(["-an", "-f", "null", "-"]);
        } else {
            if let Some(a) = plan.audio_kbps {
                cmd.args(["-map", "0:a:0", "-c:a", "aac", "-b:a", &format!("{a}k")]);
            }
            cmd.args(["-map_metadata", "0", "-movflags", "+faststart"]).arg(output);
        }

        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("No se pudo ejecutar ffmpeg: {e}"))?;
        let stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");
        *self.child.lock().unwrap() = Some(child);

        let errors = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            buf
        });

        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                progress((us / 1_000_000.0 / duration).clamp(0.0, 1.0) as f32);
            }
        }

        let status = self.child.lock().unwrap().take().map(|mut c| c.wait());
        let errors = errors.join().unwrap_or_default();
        if self.is_cancelled() {
            return Err(EncodeError::Cancelled);
        }
        match status {
            Some(Ok(s)) if s.success() => {
                progress(1.0);
                Ok(())
            }
            _ => {
                let last = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("error desconocido");
                Err(EncodeError::Failed(format!("ffmpeg falló: {last}")))
            }
        }
    }
}

#[derive(Clone)]
pub struct JobHandle {
    cancel: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl JobHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

fn file_size(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path).map(|m| m.len()).map_err(|e| format!("No se pudo leer el resultado: {e}"))
}

pub fn format_mb(mb: f64) -> String {
    if mb.fract() == 0.0 { format!("{mb:.0}") } else { format!("{mb:.1}") }
}

pub fn format_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / MIB;
    if mb >= 1024.0 { format!("{:.2} GB", mb / 1024.0) } else { format!("{mb:.1} MB") }
}

pub fn format_duration(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
