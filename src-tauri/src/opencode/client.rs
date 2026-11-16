use super::discovery::{self, Service};
use crate::config::Config;
use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::sync::RwLock;
/// Instrucciones por defecto de cada sesión. Se sustituyen por completo con
/// `~/.config/quickask/instructions.md` si existe (ver `instructions()`).
pub const INSTRUCTIONS: &str = r#"Eres "quickask", el asistente de escritorio del usuario, invocado con un atajo de teclado desde una barra flotante.
- Responde en el idioma del usuario, directo y conciso, en Markdown.
- Visión: si se adjunta pantalla.png, es la captura de la pantalla del usuario al invocarte. Si tu modelo ve imágenes, ya la estás viendo: no la abras con herramientas.
- Decide según la intención del usuario:
  1. EXPLICAR o LEER lo que hay en pantalla ("¿qué es esto?", "explícame este error", "resume esto") → responde a partir de la imagen, sin herramientas.
  2. AYUDA CON UN SISTEMA que se ve en pantalla (ticket, issue, pull request, correo, logs, base de datos…) → identifica en la captura el sistema y el identificador (URL, título, número) y usa las skills, MCP o herramientas que tengas configuradas para traer los datos reales antes de responder. Indica qué has consultado.
  3. ACTUAR EN EL NAVEGADOR ("búscame…", "rellena este formulario", "entra en…") → usa la herramienta de navegador que tengas configurada, paso a paso y como lo haría una persona. Antepón un `timeout` a cada comando y no cierres pestañas ni ventanas del usuario. Si un paso falla 2 veces, para y explica qué ha fallado.
  4. Pregunta general → responde directamente; busca en la web solo si necesitas información actual.
- Acciones con efecto hacia otras personas o sistemas (publicar, comentar, enviar correos o formularios, asignar, cambiar estados, comprar, borrar, escribir en una base de datos): NUNCA sin confirmación. Prepara el texto o la acción, muéstralo y pregunta "¿Lo hago?". Solo se ejecuta si el usuario responde "sí" explícitamente en su siguiente mensaje. Leer y consultar siempre está permitido.
- Salida de seguridad: si algo no sale tras 2 intentos razonables, no sigas probando variantes; responde con lo que tengas y qué ha fallado.
- Sé breve: la interfaz es una barra rápida."#;
/// Instrucciones propias del usuario si ha creado `instructions.md` junto a
/// `config.toml`; si no existe o está vacío, las de serie.
pub fn instructions() -> String {
    Config::path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p.with_file_name("instructions.md")).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| INSTRUCTIONS.to_owned())
}
pub struct Client {
    pub http: reqwest::Client,
    service: RwLock<Option<Service>>,
}
impl Client {
    #[cfg(test)]
    pub fn for_test(url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            service: RwLock::new(Some(Service {
                url,
                password: "synthetic-only".into(),
            })),
        }
    }
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .build()?,
            service: RwLock::new(None),
        })
    }
    pub async fn connect(&self) -> Result<()> {
        *self.service.write().await = Some(discovery::discover(&self.http).await?);
        Ok(())
    }
    async fn credentials(&self) -> Result<Service> {
        let s = self.service.read().await.clone();
        match s {
            Some(s) => Ok(s),
            None => {
                self.connect().await?;
                self.service
                    .read()
                    .await
                    .clone()
                    .context("Servicio no disponible")
            }
        }
    }
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        stream: bool,
    ) -> Result<reqwest::Response> {
        let mut service = self.credentials().await?;
        for attempt in 0..2 {
            let mut req = service.auth(&self.http, method.clone(), path);
            if !stream {
                req = req.timeout(Duration::from_secs(15));
            }
            if let Some(b) = &body {
                req = req.json(b);
            }
            match req.send().await {
                Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 => {}
                Err(e) if e.is_connect() && attempt == 0 => {}
                other => {
                    return other
                        .context("No se pudo contactar con OpenCode")?
                        .error_for_status()
                        .context("OpenCode rechazó la petición")
                }
            }
            // Retry only transport/auth failures; never resubmit an admitted prompt after an ambiguous read timeout.
            service = discovery::discover(&self.http).await?;
            *self.service.write().await = Some(service.clone());
        }
        anyhow::bail!("No se pudo autenticar con OpenCode")
    }
    pub async fn json(&self, m: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let r = self.request(m, path, body, false).await?;
        if r.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        Ok(r.json().await?)
    }
    pub async fn create_session(&self, c: &Config) -> Result<String> {
        let v = self
            .json(Method::POST, "/api/session", Some(session_body(c)?))
            .await?;
        let sid = v["data"]["id"]
            .as_str()
            .context("OpenCode no devolvió la sesión")?
            .to_owned();
        self.json(
            Method::PUT,
            &format!("/api/experimental/session/{sid}/instructions/entries/quickask"),
            Some(json!({"value":instructions()})),
        )
        .await?;
        tracing::info!(session_id=%sid,"sesión creada");
        Ok(sid)
    }
    pub async fn switch_model(&self, session: &str, config: &Config, variant: &str) -> Result<()> {
        Config::validate_variant(variant)?;
        // Preserve the session projection and messages before the model mutation.
        let backup = json!({"session":self.session(session).await?,"messages":self.list_messages(session).await?});
        let folder = crate::ctl::runtime_dir()?.join("backups");
        tokio::fs::create_dir_all(&folder).await?;
        let path = folder.join(format!("model-{}.json", uuid::Uuid::new_v4()));
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(file, &backup)?;
        let endpoint = format!("/api/session/{session}/model");
        match self
            .json(
                Method::POST,
                &endpoint,
                Some(model_body(config, variant, true)?),
            )
            .await
        {
            Ok(_) => {
                tracing::info!(
                    variant = if variant.is_empty() {
                        "default"
                    } else {
                        variant
                    },
                    "modelo de sesión actualizado (variant explícito)"
                );
                Ok(())
            }
            Err(error)
                if variant.is_empty()
                    && error
                        .downcast_ref::<reqwest::Error>()
                        .and_then(reqwest::Error::status)
                        .is_some_and(|s| {
                            s == reqwest::StatusCode::BAD_REQUEST
                                || s == reqwest::StatusCode::UNPROCESSABLE_ENTITY
                        }) =>
            {
                self.json(
                    Method::POST,
                    &endpoint,
                    Some(model_body(config, variant, false)?),
                )
                .await?;
                tracing::info!(
                    "modelo de sesión actualizado omitiendo variant por rechazo del valor default"
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
    pub async fn rename(&self, s: &str, title: &str) -> Result<()> {
        self.json(
            Method::PATCH,
            &format!("/api/session/{s}"),
            Some(json!({"title":title.chars().take(60).collect::<String>()})),
        )
        .await?;
        Ok(())
    }
    pub async fn prompt(&self, s: &str, text: &str, file: Option<&Path>) -> Result<()> {
        self.json(
            Method::POST,
            &format!("/api/session/{s}/prompt"),
            Some(prompt_body(text, file)?),
        )
        .await?;
        Ok(())
    }
    pub async fn interrupt(&self, s: &str) -> Result<()> {
        self.json(
            Method::POST,
            &format!("/api/session/{s}/interrupt"),
            Some(json!({})),
        )
        .await?;
        Ok(())
    }
    pub async fn reply_permission(&self, s: &str, id: &str, decision: &str) -> Result<()> {
        anyhow::ensure!(
            ["once", "always", "reject"].contains(&decision),
            "Decisión de permiso inválida"
        );
        anyhow::ensure!(
            id.starts_with("per_") && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "Identificador de permiso inválido"
        );
        self.json(
            Method::POST,
            &format!("/api/session/{s}/permission/{id}/reply"),
            Some(json!({"decision":decision})),
        )
        .await?;
        Ok(())
    }
    pub async fn list_permissions(&self, s: &str) -> Result<Value> {
        self.json(Method::GET, &format!("/api/session/{s}/permission"), None)
            .await
    }
    pub async fn list_messages(&self, s: &str) -> Result<Value> {
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let mut url = reqwest::Url::parse("http://localhost/")?;
            url.query_pairs_mut().append_pair("limit", "100");
            if let Some(cursor) = &cursor {
                url.query_pairs_mut().append_pair("cursor", cursor);
            } else {
                url.query_pairs_mut().append_pair("order", "desc");
            }
            let path = format!(
                "/api/session/{s}/message?{}",
                url.query().unwrap_or_default()
            );
            let page = self.json(Method::GET, &path, None).await?;
            messages.extend(
                page["data"]
                    .as_array()
                    .context("Historial OpenCode inválido")?
                    .iter()
                    .cloned(),
            );
            cursor = page["cursor"]["next"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            let Some(next) = &cursor else {
                break;
            };
            anyhow::ensure!(
                seen.insert(next.clone()),
                "OpenCode repitió el cursor del historial"
            );
        }
        Ok(json!({"data":messages}))
    }
    pub async fn session(&self, s: &str) -> Result<Value> {
        self.json(Method::GET, &format!("/api/session/{s}"), None)
            .await
    }
    /// Modelos de los proveedores indicados en `providers` más los `extra`
    /// (principal y respaldo configurados, aunque su proveedor no esté en la lista).
    pub async fn list_models(
        &self,
        providers: &[&str],
        extra: &[&str],
    ) -> Result<Vec<crate::config::ModelEntry>> {
        let v = self.json(Method::GET, "/api/model", None).await?;
        Ok(model_catalog(&v, providers, extra))
    }
}
/// Un modelo que apunta a un endpoint local solo funciona con ese servidor
/// arrancado, así que nunca se ofrece en el selector (tampoco como principal
/// o relevo configurados).
fn is_local_endpoint(m: &Value) -> bool {
    let Some(raw) = m["settings"]["baseURL"].as_str() else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    url.host_str().is_some_and(|host| {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
    })
}
pub fn model_catalog(
    v: &Value,
    providers: &[&str],
    extra: &[&str],
) -> Vec<crate::config::ModelEntry> {
    let items = v["data"]
        .as_array()
        .or(v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<crate::config::ModelEntry> = items
        .iter()
        .filter_map(|m| {
            let provider = m["providerID"].as_str()?;
            let id = format!("{provider}/{}", m["id"].as_str()?);
            if is_local_endpoint(m) {
                return None;
            }
            if !providers.contains(&provider) && !extra.contains(&id.as_str()) {
                return None;
            }
            let cost = &m["cost"][0];
            Some(crate::config::ModelEntry {
                label: m["name"].as_str().unwrap_or(&id).to_owned(),
                free: cost["input"].as_f64() == Some(0.0) && cost["output"].as_f64() == Some(0.0),
                vision: m["capabilities"]["input"]
                    .as_array()
                    .is_some_and(|i| i.iter().any(|x| x == "image")),
                variants: m["variants"]
                    .as_array()
                    .map(|vs| {
                        vs.iter()
                            .filter_map(|x| x["id"].as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                id,
            })
        })
        .collect();
    out.sort_by_key(|m| m.label.to_lowercase());
    out
}
pub fn session_body(c: &Config) -> Result<Value> {
    let (p, m) = c.model_parts()?;
    let mut model = json!({"providerID":p,"id":m});
    if !c.variant.is_empty() {
        model["variant"] = json!(c.variant);
    }
    let mut b = json!({"title":"quickask","model":model,"location":{"directory":c.workdir}});
    if !c.ask_permissions.is_empty() {
        b["permissions"] = json!(c
            .ask_permissions
            .iter()
            .map(|a| json!({"action":a,"resource":"*","effect":"ask"}))
            .collect::<Vec<_>>());
    }
    Ok(b)
}
pub fn model_body(config: &Config, variant: &str, explicit_default: bool) -> Result<Value> {
    Config::validate_variant(variant)?;
    let (provider, id) = config.model_parts()?;
    let mut model = json!({"providerID":provider,"id":id});
    if !variant.is_empty() || explicit_default {
        model["variant"] = json!(if variant.is_empty() {
            "default"
        } else {
            variant
        });
    }
    Ok(json!({"model":model}))
}
pub fn prompt_body(text: &str, file: Option<&Path>) -> Result<Value> {
    let mut b = json!({"text":text});
    if let Some(p) = file {
        let uri = reqwest::Url::from_file_path(p)
            .map_err(|_| anyhow::anyhow!("Ruta de captura inválida"))?;
        b["files"] = json!([{"uri":uri.as_str(),"name":"pantalla.png","description":"Captura de la pantalla del usuario"}]);
    }
    Ok(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_instructions_match_phase_two_snapshot() {
        assert_eq!(
            INSTRUCTIONS,
            include_str!("../../tests/fixtures/session-instructions.txt").trim_end_matches('\n')
        );
    }

    #[test]
    fn catalog_keeps_listed_providers_and_configured_extras() {
        let v = json!({"data":[
            {"providerID":"opencode-go","id":"kimi-k3","name":"Kimi K3","capabilities":{"input":["text","image"]},"variants":[{"id":"max"}],"cost":[{"input":3,"output":9}]},
            {"providerID":"opencode","id":"deepseek-v4.1-flash-free","name":"DeepSeek V4.1 Flash Free","capabilities":{"input":["text","image"]},"variants":[{"id":"low"},{"id":"high"}],"cost":[{"input":0,"output":0}]},
            {"providerID":"ollama-cloud","id":"glm-5.3","name":"GLM-5.3","capabilities":{"input":["text"]},"cost":[{"input":1.4,"output":4.4}]},
            {"providerID":"openai","id":"gpt-6","name":"GPT-6","capabilities":{"input":["text"]},"cost":[{"input":1,"output":2}]},
            {"providerID":"ollama","id":"llava:latest","name":"LLaVA","settings":{"baseURL":"http://127.0.0.1:11434/v1"},"capabilities":{"input":["text","image"]},"cost":[]}
        ]});
        let c = model_catalog(
            &v,
            &["opencode-go", "ollama-cloud"],
            &["opencode/deepseek-v4.1-flash-free", "ollama/llava:latest"],
        );
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].id, "opencode/deepseek-v4.1-flash-free");
        assert!(c[0].free && c[0].vision);
        assert_eq!(c[0].variants, ["low", "high"]);
        assert_eq!(c[1].id, "ollama-cloud/glm-5.3");
        assert!(!c[1].free);
        assert_eq!(c[2].label, "Kimi K3");
        assert!(!c[2].free);
        // Los modelos de un servidor local no salen ni aunque sean el principal.
        assert!(c.iter().all(|m| m.id != "ollama/llava:latest"));
        // Sin el proveedor en la lista, sus modelos no salen.
        let only_go = model_catalog(&v, &["opencode-go"], &[]);
        assert!(only_go.iter().all(|m| m.id.starts_with("opencode-go/")));
    }
    #[test]
    fn local_endpoints_are_detected() {
        for base in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:11434/v1",
            "http://[::1]:11434/v1",
            "http://0.0.0.0:8080/v1",
        ] {
            assert!(
                is_local_endpoint(&json!({"settings":{"baseURL":base}})),
                "{base}"
            );
        }
        for base in ["https://ollama.com/v1", "https://api.example.com/v1"] {
            assert!(
                !is_local_endpoint(&json!({"settings":{"baseURL":base}})),
                "{base}"
            );
        }
        assert!(!is_local_endpoint(&json!({})));
    }
    #[test]
    fn bodies() {
        let c = Config {
            model: "opencode-go/deepseek-v4.1-flash".into(),
            fallback_model: "opencode/deepseek-v4.1-flash-free".into(),
            workdir: "/tmp".into(),
            ask_permissions: vec!["shell".into()],
            variant: "high".into(),
            ..Default::default()
        };
        assert_eq!(
            session_body(&c).unwrap(),
            json!({"title":"quickask","model":{"providerID":"opencode-go","id":"deepseek-v4.1-flash","variant":"high"},"location":{"directory":"/tmp"},"permissions":[{"action":"shell","resource":"*","effect":"ask"}]})
        );
        let fallback = Config {
            model: c.fallback_model.clone(),
            ..c.clone()
        };
        assert_eq!(
            session_body(&fallback).unwrap()["model"],
            json!({"providerID":"opencode","id":"deepseek-v4.1-flash-free","variant":"high"})
        );
        assert_eq!(prompt_body("hola", None).unwrap(), json!({"text":"hola"}));
        assert_eq!(
            prompt_body("mira", Some(Path::new("/tmp/test shot.png"))).unwrap(),
            json!({"text":"mira","files":[{"uri":"file:///tmp/test%20shot.png","name":"pantalla.png","description":"Captura de la pantalla del usuario"}]})
        );
    }
    #[test]
    fn switch_model_bodies_use_explicit_high_and_default() {
        let c = Config {
            model: "provider/id/with/slash".into(),
            ..Default::default()
        };
        assert_eq!(
            model_body(&c, "high", true).unwrap(),
            json!({"model":{"providerID":"provider","id":"id/with/slash","variant":"high"}})
        );
        assert_eq!(
            model_body(&c, "", true).unwrap(),
            json!({"model":{"providerID":"provider","id":"id/with/slash","variant":"default"}})
        );
        assert_eq!(
            model_body(&c, "", false).unwrap(),
            json!({"model":{"providerID":"provider","id":"id/with/slash"}})
        );
        assert!(model_body(&c, "Bad!", true).is_err());
    }
    #[tokio::test]
    async fn message_history_paginates_for_backups_and_reconciliation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::for_test(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            for page in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0u8; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.lines().next().unwrap();
                if page == 0 {
                    assert!(path.contains("order=desc"));
                } else {
                    assert!(path.contains("cursor=a%26b%3F"));
                    assert!(!path.contains("order="));
                }
                let body = if page == 0 {
                    json!({"data":[{"id":"new"}],"cursor":{"next":"a&b?"}})
                } else {
                    json!({"data":[{"id":"old"}],"cursor":{"next":null}})
                }
                .to_string();
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let result = client.list_messages("ses_test").await.unwrap();
        server.await.unwrap();
        assert_eq!(result["data"], json!([{"id":"new"},{"id":"old"}]));
    }
}
