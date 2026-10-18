use std::path::{Path, PathBuf};
use std::process::Stdio;

use ffmpeg_sidecar::download::{
    FfmpegDownloadProgressEvent, download_ffmpeg_package_with_progress, ffmpeg_download_url,
    unpack_ffmpeg_without_extras,
};

const EXE: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

fn download_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    };
    base.map(|b| b.join("CompresorVideo"))
}

fn works(path: &Path) -> bool {
    crate::encoder::command(path)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn locate() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(EXE));
        candidates.push(dir.join("../Resources").join(EXE));
    }
    if let Some(dir) = download_dir() {
        candidates.push(dir.join(EXE));
    }
    if cfg!(target_os = "macos") {
        candidates.push("/opt/homebrew/bin/ffmpeg".into());
        candidates.push("/usr/local/bin/ffmpeg".into());
    }
    candidates.push(EXE.into());
    candidates.into_iter().find(|c| works(c))
}

pub fn download(progress: impl Fn(f32)) -> Result<PathBuf, String> {
    let dir = download_dir().ok_or("No se encontró la carpeta de datos del usuario")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let url = ffmpeg_download_url().map_err(|e| e.to_string())?;
    let archive = download_ffmpeg_package_with_progress(url, &dir, |event| {
        if let FfmpegDownloadProgressEvent::Downloading { total_bytes, downloaded_bytes } = event
            && total_bytes > 0
        {
            progress(downloaded_bytes as f32 / total_bytes as f32);
        }
    })
    .map_err(|e| format!("Error descargando ffmpeg: {e}"))?;
    unpack_ffmpeg_without_extras(&archive, &dir).map_err(|e| format!("Error descomprimiendo ffmpeg: {e}"))?;
    let path = dir.join(EXE);
    if works(&path) { Ok(path) } else { Err("ffmpeg se descargó pero no arranca".into()) }
}
