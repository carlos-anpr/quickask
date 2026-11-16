use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize, Deserialize)]
pub struct Command {
    pub cmd: String,
    #[serde(default)]
    pub activation_token: Option<String>,
}
pub fn runtime_dir() -> Result<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR absent")?)
        .join("quickask");
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}
pub fn send(path: &Path, cmd: &Command) -> Result<()> {
    let mut s = UnixStream::connect(path)?;
    s.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;
    serde_json::to_writer(&mut s, cmd)?;
    s.write_all(b"\n")?;
    Ok(())
}
pub fn bind(path: &Path) -> Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            bail!("quickask ya está ejecutándose");
        }
        std::fs::remove_file(path)?;
    }
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}
pub struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl SocketGuard {
    pub fn new(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        Ok(Self {
            path: path.into(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn cleanup(&self) -> Result<()> {
        match std::fs::symlink_metadata(&self.path) {
            Ok(m) if m.dev() == self.device && m.ino() == self.inode => {
                std::fs::remove_file(&self.path)?
            }
            Ok(_) => {} // A successor already owns this pathname.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
pub fn receive(s: UnixStream) -> Result<Command> {
    s.set_read_timeout(Some(std::time::Duration::from_secs(1)))?;
    let mut line = String::new();
    BufReader::new(s).take(8192).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ctl.sock");
        let l = bind(&p).unwrap();
        send(
            &p,
            &Command {
                cmd: "toggle".into(),
                activation_token: Some("token".into()),
            },
        )
        .unwrap();
        let c = receive(l.accept().unwrap().0).unwrap();
        assert_eq!(c.cmd, "toggle");
        assert_eq!(c.activation_token.as_deref(), Some("token"));
        assert_eq!(
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn guard_cleans_up_its_socket_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ctl.sock");
        let _listener = bind(&p).unwrap();
        {
            let _guard = SocketGuard::new(&p).unwrap();
        }
        assert!(!p.exists());
    }
    #[test]
    fn old_guard_does_not_unlink_a_successors_socket() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ctl.sock");
        let _old_listener = bind(&p).unwrap();
        let guard = SocketGuard::new(&p).unwrap();
        std::fs::remove_file(&p).unwrap();
        let _new_listener = bind(&p).unwrap();
        guard.cleanup().unwrap();
        drop(guard);
        assert!(UnixStream::connect(&p).is_ok());
    }
}
