use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};
pub struct ScreenFile {
    pub path: PathBuf,
}
impl Drop for ScreenFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
#[derive(Clone)]
pub struct Snapshot(
    tokio::sync::watch::Receiver<Option<std::result::Result<std::sync::Arc<ScreenFile>, String>>>,
);
impl Snapshot {
    pub async fn finish(mut self) -> Result<std::sync::Arc<ScreenFile>> {
        loop {
            if let Some(result) = self.0.borrow_and_update().clone() {
                return result.map_err(anyhow::Error::msg);
            }
            self.0.changed().await.context("Captura interrumpida")?;
        }
    }
}
const SCREENSHOT_NAME: &str = "org.gnome.Screenshot";
static BUS: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);

fn connect(bus: &mut Option<zbus::blocking::Connection>) -> Result<&zbus::blocking::Connection> {
    if bus.is_none() {
        *bus = Some(
            zbus::blocking::connection::Builder::session()?
                .method_timeout(Duration::from_secs(2))
                .build()?,
        );
    }
    bus.as_ref().context("Conexión D-Bus no disponible")
}
fn with_screenshot_name<T>(
    action: impl FnOnce(&zbus::blocking::Connection) -> Result<T>,
) -> Result<T> {
    let mut bus = BUS
        .lock()
        .map_err(|_| anyhow::anyhow!("Conexión D-Bus bloqueada"))?;
    let connection = connect(&mut bus)?;
    let reply = connection.request_name_with_flags(
        SCREENSHOT_NAME,
        zbus::fdo::RequestNameFlags::DoNotQueue.into(),
    )?;
    anyhow::ensure!(
        matches!(
            reply,
            zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner
        ),
        "Nombre de captura ocupado"
    );
    let result = action(connection);
    // Retaining the name breaks the user's gnome-screenshot (verified in B5).
    // Release even after AccessDenied or a failed screenshot; reuse only the connection.
    if let Err(error) = connection.release_name(SCREENSHOT_NAME) {
        *bus = None; // Closing this connection also relinquishes any retained name.
        return Err(error.into());
    }
    result
}
pub fn initialize() {
    if let Err(error) = with_screenshot_name(|_| Ok(())) {
        tracing::warn!(error=%error, "captura D-Bus no disponible al arrancar; se reintentará");
    }
}
pub fn shutdown() {
    if let Ok(mut bus) = BUS.lock() {
        if let Some(connection) = bus.take() {
            if let Err(error) = connection.release_name(SCREENSHOT_NAME) {
                tracing::warn!(error=%error, "no se pudo liberar el nombre D-Bus");
            }
            if let Err(error) = connection.close() {
                tracing::warn!(error=%error, "no se pudo cerrar la conexión D-Bus");
            }
            tracing::info!("conexión de captura D-Bus cerrada");
        }
    }
}
fn dbus_capture(path: &Path) -> Result<()> {
    with_screenshot_name(|connection| {
        let filename = path.to_str().context("Ruta de captura inválida")?;
        let reply = connection.call_method(
            Some("org.gnome.Shell.Screenshot"),
            "/org/gnome/Shell/Screenshot",
            Some("org.gnome.Shell.Screenshot"),
            "Screenshot",
            &(false, false, filename),
        )?;
        let (success, returned): (bool, String) = reply.body().deserialize()?;
        anyhow::ensure!(
            success && returned == filename && path.is_file(),
            "GNOME no devolvió la captura"
        );
        Ok(())
    })
}
enum Capture {
    Ready(PathBuf),
    Process {
        path: PathBuf,
        child: Child,
        started: Instant,
    },
}
impl Capture {
    fn finish(self) -> Result<PathBuf> {
        match self {
            Self::Ready(path) => Ok(path),
            Self::Process {
                path,
                mut child,
                started,
            } => {
                let status = child.wait();
                tracing::info!(
                    method = "gnome-screenshot",
                    ms = started.elapsed().as_millis(),
                    "capture"
                );
                if !status?.success() {
                    let _ = std::fs::remove_file(&path);
                    anyhow::bail!("No se pudo capturar la pantalla");
                }
                Ok(path)
            }
        }
    }
}
fn launch() -> Result<Capture> {
    let dir = crate::ctl::runtime_dir()?.join("shots");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    let started = Instant::now();
    match dbus_capture(&path) {
        Ok(()) => {
            tracing::info!(
                method = "dbus",
                ms = started.elapsed().as_millis(),
                "capture"
            );
            return Ok(Capture::Ready(path));
        }
        Err(error) => {
            tracing::warn!(error=%error, "captura D-Bus fallida; usando gnome-screenshot")
        }
    }
    let _ = std::fs::remove_file(&path);
    let started = Instant::now();
    let child = Command::new("gnome-screenshot")
        .arg("-f")
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("No se pudo iniciar la captura de pantalla")?;
    Ok(Capture::Process {
        path,
        child,
        started,
    })
}
pub fn start_capture() -> Result<Snapshot> {
    // D=0: no timer. D-Bus completes the snapshot before the window maps (~77 ms).
    // On fallback, launch the slower subprocess before show, as measured in S2.
    let capture = launch()?;
    let (tx, rx) = tokio::sync::watch::channel(None);
    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            capture
                .finish()
                .map(|path| std::sync::Arc::new(ScreenFile { path }))
        })
        .await;
        let result = match result {
            Ok(r) => r.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Some(result));
    });
    Ok(Snapshot(rx))
}
pub async fn capture() -> Result<PathBuf> {
    tokio::task::spawn_blocking(|| launch()?.finish()).await?
}
pub fn thumbnail(path: &Path) -> Result<String> {
    let image = image::open(path)?.thumbnail(320, 320);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Jpeg)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(bytes.into_inner())
    ))
}
pub fn cleanup() -> Result<()> {
    let dir = crate::ctl::runtime_dir()?.join("shots");
    std::fs::create_dir_all(&dir)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file()
            && SystemTime::now()
                .duration_since(meta.modified()?)
                .unwrap_or_default()
                > Duration::from_secs(86400)
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnail_size() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("test.png");
        image::RgbImage::from_pixel(1920, 1080, image::Rgb([255, 0, 255]))
            .save(&p)
            .unwrap();
        let t = thumbnail(&p).unwrap();
        let bytes = STANDARD
            .decode(t.strip_prefix("data:image/jpeg;base64,").unwrap())
            .unwrap();
        let im = image::load_from_memory(&bytes).unwrap();
        assert_eq!(im.width(), 320);
        assert!(im.height() <= 320);
    }
}
