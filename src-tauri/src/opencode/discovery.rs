use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Clone, Deserialize)]
pub struct Service {
    pub url: String,
    pub password: String,
}
impl Service {
    pub async fn read() -> Result<Self> {
        let path = dirs::state_dir()
            .context("No se encuentra el directorio de estado")?
            .join("opencode/service.json");
        let bytes = tokio::fs::read(path)
            .await
            .context("OpenCode no está disponible")?;
        let s: Self = serde_json::from_slice(&bytes).context("service.json inválido")?;
        let url = reqwest::Url::parse(&s.url)?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
        {
            bail!("La dirección del servicio OpenCode debe ser local");
        }
        Ok(s)
    }
    pub fn auth(
        &self,
        http: &reqwest::Client,
        method: reqwest::Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        http.request(
            method,
            format!("{}{}", self.url.trim_end_matches('/'), path),
        )
        .basic_auth("opencode", Some(&self.password))
    }
}
pub async fn discover(http: &reqwest::Client) -> Result<Service> {
    if let Ok(s) = Service::read().await {
        if healthy(http, &s).await {
            return Ok(s);
        }
    }
    let exe = dirs::home_dir()
        .map(|h| h.join(".opencode/bin/opencode"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("opencode"));
    let mut child = tokio::process::Command::new(exe)
        .args(["service", "start"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("No se pudo iniciar OpenCode")?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(s) = Service::read().await {
            if healthy(http, &s).await {
                return Ok(s);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    bail!("No se pudo conectar con OpenCode en 10 segundos")
}
async fn healthy(http: &reqwest::Client, s: &Service) -> bool {
    s.auth(http, reqwest::Method::GET, "/api/info")
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_service() {
        let s:Service=serde_json::from_str(r#"{"url":"http://127.0.0.1:12345","password":"synthetic-only","pid":123,"version":"2"}"#).unwrap();
        assert_eq!(s.url, "http://127.0.0.1:12345");
        assert_eq!(s.password, "synthetic-only");
        assert!(serde_json::from_str::<Service>("{}").is_err());
    }
}
