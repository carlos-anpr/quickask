use crate::{
    config::{self, Config},
    ctl,
    events::UiEvent,
    heuristics,
    opencode::client::Client,
    screen,
};
use anyhow::{Context, Result};
use chrono::Timelike;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{Emitter, Manager, State};
use tokio::sync::watch;
struct Active {
    id: String,
    cancel: watch::Sender<bool>,
}
struct Conversation {
    session: Option<String>,
    active: Option<Active>,
    first: bool,
    backend: String,
    variant: String,
    model: String,
    fallback_model: String,
}
pub struct AppState {
    config: Config,
    config_path: std::path::PathBuf,
    client: Client,
    inner: Mutex<Conversation>,
    session_gate: tokio::sync::Mutex<()>,
    shot: Mutex<Option<screen::Snapshot>>,
    /// Catálogo de modelos de OpenCode (caché de 5 min).
    catalog: tokio::sync::Mutex<Option<(Instant, Vec<config::ModelEntry>)>>,
    /// La captura de esta invocación aún no se ha adjuntado: en modo auto va con la primera pregunta.
    fresh_shot: std::sync::atomic::AtomicBool,
    shown: Mutex<Option<Instant>>,
}
impl AppState {
    fn new(config: Config) -> Result<Self> {
        Self::with_config_path(config, Config::path()?)
    }
    fn with_config_path(config: Config, config_path: std::path::PathBuf) -> Result<Self> {
        let variant = config.variant.clone();
        let model = config.model.clone();
        let fallback_model = config.fallback_model.clone();
        Ok(Self {
            config,
            config_path,
            client: Client::new()?,
            inner: Mutex::new(Conversation {
                session: None,
                active: None,
                first: true,
                backend: "connecting".into(),
                variant,
                model,
                fallback_model,
            }),
            session_gate: tokio::sync::Mutex::new(()),
            shot: Mutex::new(None),
            catalog: tokio::sync::Mutex::new(None),
            fresh_shot: std::sync::atomic::AtomicBool::new(false),
            shown: Mutex::new(None),
        })
    }
    fn config_snapshot(&self) -> Result<Config> {
        let mut config = self.config.clone();
        let inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
        config.variant = inner.variant.clone();
        config.model = inner.model.clone();
        config.fallback_model = inner.fallback_model.clone();
        Ok(config)
    }
    fn screen_for_question(&self, mode: &str, text: &str) -> bool {
        // Preserve 1fc886b: the first question consumes the fresh invocation,
        // including when screen is explicitly off or automatic capture is disabled.
        let fresh = self
            .fresh_shot
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        mode == "on"
            || (mode == "auto"
                && self.config.screen_auto
                && (fresh || heuristics::needs_screen(text)))
    }
    async fn catalog(&self) -> Vec<config::ModelEntry> {
        let mut cache = self.catalog.lock().await;
        if let Some((at, models)) = cache.as_ref() {
            if at.elapsed() < std::time::Duration::from_secs(300) && !models.is_empty() {
                return models.clone();
            }
        }
        let (model, fallback) = match self.inner.lock() {
            Ok(i) => (i.model.clone(), i.fallback_model.clone()),
            Err(_) => return Vec::new(),
        };
        let extra = [model.as_str(), fallback.as_str(), config::DEFAULT_FALLBACK];
        let providers: Vec<&str> = self
            .config
            .model_providers
            .iter()
            .map(String::as_str)
            .collect();
        match self.client.list_models(&providers, &extra).await {
            Ok(models) => {
                *cache = Some((Instant::now(), models.clone()));
                models
            }
            Err(e) => {
                tracing::warn!(error=%e, "no se pudo leer el catálogo de modelos");
                cache.as_ref().map(|(_, m)| m.clone()).unwrap_or_default()
            }
        }
    }
    async fn change_model(&self, model: &str) -> Result<()> {
        let fallback = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
            if inner.fallback_model == model {
                inner.model.clone()
            } else {
                inner.fallback_model.clone()
            }
        };
        let fallback = fallback.as_str();
        let _gate = self.session_gate.lock().await;
        // Se valida y se prepara el documento antes de tocar la sesión.
        let updated = Config::model_document(
            &std::fs::read_to_string(&self.config_path)?,
            model,
            fallback,
        )?;
        let (session, variant) = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
            (inner.session.clone(), inner.variant.clone())
        };
        if let Some(session) = &session {
            let mut config = self.config.clone();
            config.model = model.into();
            self.client.switch_model(session, &config, &variant).await?;
        }
        Config::write_at(&self.config_path, &updated)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
        inner.model = model.into();
        inner.fallback_model = fallback.into();
        tracing::info!(model, fallback_model = fallback, "modelo cambiado");
        Ok(())
    }
    async fn change_variant(&self, variant: &str) -> Result<()> {
        Config::validate_variant(variant)?;
        let _gate = self.session_gate.lock().await;
        // Validate persistence before modifying the session; re-read when saving
        // so unrelated settings edited meanwhile are retained.
        Config::variant_document(&std::fs::read_to_string(&self.config_path)?, variant)?;
        let (session, previous) = {
            let inner = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
            (inner.session.clone(), inner.variant.clone())
        };
        let current = self.config_snapshot()?;
        if let Some(session) = &session {
            self.client.switch_model(session, &current, variant).await?;
        }
        if let Err(save_error) = Config::save_variant_at(&self.config_path, variant) {
            if let Some(session) = &session {
                if let Err(rollback_error) =
                    self.client.switch_model(session, &current, &previous).await
                {
                    tracing::error!(error=%rollback_error, "no se pudo restaurar el esfuerzo anterior");
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.variant = variant.into();
                    }
                }
            }
            return Err(save_error);
        }
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("Estado no disponible"))?
            .variant = variant.into();
        Ok(())
    }
    async fn session(&self, app: &tauri::AppHandle) -> Result<String> {
        let _guard = self.session_gate.lock().await;
        if let Some(s) = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("Estado no disponible"))?
            .session
            .clone()
        {
            return Ok(s);
        }
        self.backend(app, "connecting");
        match self.client.create_session(&self.config_snapshot()?).await {
            Ok(s) => {
                if let Ok(mut inner) = self.inner.lock() {
                    inner.session = Some(s.clone());
                    inner.first = true;
                }
                self.backend(app, "ready");
                Ok(s)
            }
            Err(e) => {
                self.backend(app, "down");
                Err(e)
            }
        }
    }
    fn backend(&self, app: &tauri::AppHandle, status: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.backend = status.into();
        }
        emit(
            app,
            UiEvent::Backend {
                status: status.into(),
                detail: None,
            },
        );
    }
}
fn emit(app: &tauri::AppHandle, event: UiEvent) {
    let _ = app.emit("qa://event", event);
}
fn error(e: impl std::fmt::Display) -> String {
    tracing::error!(error=%e,"operación fallida");
    "No se pudo completar la operación. Revisa el registro de quickask".into()
}
fn window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow> {
    app.get_webview_window("main")
        .context("Ventana no disponible")
}
pub fn show(app: &tauri::AppHandle, token: Option<&str>) {
    let start = Instant::now();
    tracing::info!("toggle recibido");
    let state = app.state::<Arc<AppState>>();
    if let Ok(w) = window(app) {
        if let Ok(mut shot) = state.shot.lock() {
            *shot = None;
            match screen::start_capture() {
                Ok(s) => {
                    *shot = Some(s);
                    state
                        .fresh_shot
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                Err(e) => tracing::warn!(error=%e,"precaptura fallida"),
            }
        }
        if let Some(t) = token {
            if let Ok(gtk) = w.gtk_window() {
                use gtk::prelude::GtkWindowExt;
                gtk.set_startup_id(t);
            }
        }
        if let Ok(mut shown) = state.shown.lock() {
            *shown = Some(Instant::now());
        }
        let _ = w.show();
        let _ = w.center();
        let _ = w.set_focus();
        emit(
            app,
            UiEvent::Summoned {
                greeting: heuristics::greeting(chrono::Local::now().hour()),
            },
        );
        tracing::info!(ms = start.elapsed().as_millis(), "toggle visible");
    }
}
#[tauri::command]
pub fn hide(app: tauri::AppHandle) -> Result<(), String> {
    window(&app).map_err(error)?.hide().map_err(error)?;
    let state = app.state::<Arc<AppState>>();
    if let Ok(mut s) = state.shot.lock() {
        *s = None;
    }
    emit(&app, UiEvent::Hidden);
    Ok(())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResult {
    turn_id: String,
}
#[tauri::command(rename_all = "camelCase")]
pub async fn ask(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    text: String,
    screen: String,
) -> Result<AskResult, String> {
    if text.trim().is_empty() {
        return Err("Escribe una pregunta".into());
    }
    if !["auto", "on", "off"].contains(&screen.as_str()) {
        return Err("Modo de pantalla inválido".into());
    }
    let turn = uuid::Uuid::new_v4().to_string();
    let (tx, mut rx) = watch::channel(false);
    {
        let mut inner = state.inner.lock().map_err(error)?;
        if inner.active.is_some() {
            return Err("busy".into());
        }
        inner.active = Some(Active {
            id: turn.clone(),
            cancel: tx,
        });
    }
    let with_screen = state.screen_for_question(&screen, &text);
    emit(
        &app,
        UiEvent::TurnStarted {
            turn_id: turn.clone(),
            text: text.clone(),
            screen: with_screen,
        },
    );
    let state = state.inner().clone();
    let id = turn.clone();
    tauri::async_runtime::spawn(async move {
        let start = Instant::now();
        let first = std::sync::atomic::AtomicBool::new(true);
        let sink = |event: UiEvent| {
            let current = state
                .inner
                .lock()
                .map(|i| i.active.as_ref().is_some_and(|a| a.id == id))
                .unwrap_or(false);
            if current {
                if matches!(event, UiEvent::Text { .. })
                    && first.swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    tracing::info!(ms = start.elapsed().as_millis(), "primer delta desde ask");
                }
                if let UiEvent::Backend { status, .. } = &event {
                    if let Ok(mut inner) = state.inner.lock() {
                        inner.backend = status.clone();
                    }
                }
                emit(&app, event);
            }
        };
        let shot = state.shot.lock().ok().and_then(|s| s.clone());
        let file = if with_screen {
            sink(UiEvent::Screen {
                turn_id: id.clone(),
                status: "capturing".into(),
                thumb: None,
                error: None,
            });
            let captured = match shot {
                Some(s) => s.finish().await,
                None => screen::capture()
                    .await
                    .map(|path| Arc::new(screen::ScreenFile { path })),
            };
            match captured {
                Ok(p) => {
                    let path = p.path.clone();
                    let thumb = tokio::task::spawn_blocking(move || screen::thumbnail(&path))
                        .await
                        .ok()
                        .and_then(Result::ok);
                    sink(UiEvent::Screen {
                        turn_id: id.clone(),
                        status: "attached".into(),
                        thumb,
                        error: None,
                    });
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(error=%e,"captura fallida");
                    sink(UiEvent::Screen {
                        turn_id: id.clone(),
                        status: "failed".into(),
                        thumb: None,
                        error: Some("No se pudo capturar la pantalla".into()),
                    });
                    None
                }
            }
        } else {
            drop(shot);
            None
        };
        let result: Result<(bool, String)> = async {
            let session = state.session(&app).await?;
            let rename = {
                let mut inner = state
                    .inner
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Estado no disponible"))?;
                let first = inner.first;
                inner.first = false;
                first
            };
            if rename {
                let _ = state.client.rename(&session, &text).await;
            }
            if *rx.borrow() {
                sink(UiEvent::Done {
                    turn_id: id.clone(),
                    outcome: "interrupted".into(),
                    error: None,
                });
                return Ok((false, String::new()));
            }
            crate::engine::execute(
                &state.client,
                &state
                    .config_snapshot()
                    .map_err(|_| anyhow::anyhow!("Estado no disponible"))?,
                &crate::engine::Turn {
                    session: &session,
                    id: &id,
                    text: &text,
                    file: file.as_ref().map(|f| f.path.as_path()),
                },
                &mut rx,
                &sink,
            )
            .await
        }
        .await;
        drop(file);
        if let Err(e) = &result {
            tracing::error!(error=%e,"turno fallido");
            sink(UiEvent::Done {
                turn_id: id.clone(),
                outcome: "failed".into(),
                error: Some("No se pudo completar la respuesta".into()),
            });
        }
        let current = if let Ok(mut inner) = state.inner.lock() {
            if inner.active.as_ref().is_some_and(|a| a.id == id) {
                inner.active = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if current
            && state.config.notify_when_hidden
            && window(&app)
                .and_then(|w| Ok(w.is_visible()?))
                .map(|v| !v)
                .unwrap_or(false)
        {
            let message = result
                .ok()
                .map(|(_, s)| s.chars().take(120).collect::<String>())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Respuesta completada".into());
            let _ = tokio::process::Command::new("notify-send")
                .args(["-a", "quickask", "quickask", &message])
                .status()
                .await;
        }
        tracing::info!(ms=start.elapsed().as_millis(),turn_id=%id,"turno terminado");
    });
    Ok(AskResult { turn_id: turn })
}
#[tauri::command]
pub async fn stop(app: tauri::AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let current = {
        let inner = state.inner.lock().map_err(error)?;
        inner
            .active
            .as_ref()
            .map(|a| (inner.session.clone(), a.id.clone(), a.cancel.clone()))
    };
    if let Some((session, id, cancel)) = current {
        if let Some(s) = session {
            let state = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state.client.interrupt(&s).await {
                    tracing::warn!(error=%e,"interrupt fallido");
                }
            });
        }
        let state = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let active = if let Ok(mut inner) = state.inner.lock() {
                if inner.active.as_ref().is_some_and(|a| a.id == id) {
                    inner.active = None;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let _ = cancel.send(true);
            if active {
                emit(
                    &app,
                    UiEvent::Done {
                        turn_id: id,
                        outcome: "interrupted".into(),
                        error: None,
                    },
                );
            }
        });
    }
    Ok(())
}
#[tauri::command(rename = "replyPermission", rename_all = "camelCase")]
pub async fn reply_permission(
    state: State<'_, Arc<AppState>>,
    request_id: String,
    decision: String,
) -> Result<(), String> {
    let s = state
        .inner
        .lock()
        .map_err(error)?
        .session
        .clone()
        .ok_or("No hay conversación")?;
    state
        .client
        .reply_permission(&s, &request_id, &decision)
        .await
        .map_err(error)
}
#[tauri::command(rename = "newConversation")]
pub async fn new_conversation(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let _gate = state.session_gate.lock().await;
    let old = {
        let mut inner = state.inner.lock().map_err(error)?;
        if let Some(active) = inner.active.take() {
            let _ = active.cancel.send(true);
        }
        inner.first = true;
        inner.session.take()
    };
    if let Some(s) = old {
        let _ = state.client.interrupt(&s).await;
    }
    emit(&app, UiEvent::ConversationReset);
    state.backend(&app, "connecting");
    match state
        .client
        .create_session(&state.config_snapshot().map_err(error)?)
        .await
    {
        Ok(s) => {
            state.inner.lock().map_err(error)?.session = Some(s);
            state.backend(&app, "ready");
            Ok(())
        }
        Err(e) => {
            state.backend(&app, "down");
            Err(error(e))
        }
    }
}
#[tauri::command]
pub fn resize(height: f64) -> Result<(), String> {
    if !height.is_finite() || height < 0. {
        return Err("Altura inválida".into());
    }
    Ok(())
}
#[tauri::command(rename = "needsScreen")]
pub fn needs_screen(text: String) -> bool {
    heuristics::needs_screen(&text)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    model: String,
    fallback_model: String,
    models: Vec<config::ModelEntry>,
    variant: String,
    session_id: Option<String>,
    backend: String,
    shortcut: String,
    theme: String,
    busy: bool,
}
#[tauri::command(rename = "getState")]
pub async fn get_state(state: State<'_, Arc<AppState>>) -> Result<Snapshot, String> {
    let models = state.catalog().await;
    let inner = state.inner.lock().map_err(error)?;
    Ok(Snapshot {
        model: inner.model.clone(),
        fallback_model: inner.fallback_model.clone(),
        models,
        variant: inner.variant.clone(),
        session_id: inner.session.clone(),
        backend: inner.backend.clone(),
        shortcut: state.config.shortcut.clone(),
        theme: state.config.theme.clone(),
        busy: inner.active.is_some(),
    })
}
#[tauri::command(rename = "setVariant", rename_all = "camelCase")]
pub async fn set_variant(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    variant: String,
) -> Result<(), String> {
    Config::validate_variant(&variant).map_err(|e| e.to_string())?;
    let model = state.inner.lock().map_err(error)?.model.clone();
    if !variant.is_empty()
        && state
            .catalog()
            .await
            .iter()
            .find(|m| m.id == model)
            .is_some_and(|m| !m.variants.contains(&variant))
    {
        return Err("Este modelo no admite ese nivel de esfuerzo".into());
    }
    let result = state.change_variant(&variant).await;
    let current = state.inner.lock().map_err(error)?.variant.clone();
    emit(&app, UiEvent::Variant { variant: current });
    result.map_err(error)
}
#[tauri::command(rename = "setModel", rename_all = "camelCase")]
pub async fn set_model(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    model: String,
) -> Result<(), String> {
    config::validate_model(&model).map_err(|e| e.to_string())?;
    let catalog = state.catalog().await;
    let entry = catalog.iter().find(|m| m.id == model);
    if !catalog.is_empty() && entry.is_none() {
        return Err("Modelo no disponible".into());
    }
    let variant = state.inner.lock().map_err(error)?.variant.clone();
    if !variant.is_empty() && entry.is_some_and(|m| !m.variants.contains(&variant)) {
        // El esfuerzo actual no existe en el nuevo modelo: vuelve a "Estándar".
        let reset = state.change_variant("").await;
        let current = state.inner.lock().map_err(error)?.variant.clone();
        emit(&app, UiEvent::Variant { variant: current });
        reset.map_err(error)?;
    }
    let result = state.change_model(&model).await;
    let inner = state.inner.lock().map_err(error)?;
    emit(
        &app,
        UiEvent::Model {
            model: inner.model.clone(),
            fallback_model: inner.fallback_model.clone(),
        },
    );
    result.map_err(error)
}
#[tauri::command(rename = "openInTerminal")]
pub async fn open_in_terminal(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let session = state
        .inner
        .lock()
        .map_err(error)?
        .session
        .clone()
        .ok_or("No hay conversación")?;
    let mut child = tokio::process::Command::new("gnome-terminal")
        .arg(format!("--working-directory={}", state.config.workdir))
        .args(["--", "opencode", "--session", &session])
        .spawn()
        .map_err(error)?;
    tauri::async_runtime::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}
/// Solo enlaces web y de correo: nada de `file:`, `javascript:` ni rutas locales.
fn openable_url(url: &str) -> Option<&str> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let ok = (lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:"))
        && !url.chars().any(char::is_control);
    ok.then_some(url)
}
#[tauri::command(rename = "openUrl")]
pub async fn open_url(url: String) -> Result<(), String> {
    let url = openable_url(&url).ok_or("Enlace no admitido")?;
    let mut child = tokio::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(error)?;
    tauri::async_runtime::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}
pub struct ShutdownSignals {
    terminate: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}
pub fn shutdown_signals() -> std::io::Result<ShutdownSignals> {
    // Register before binding ctl.sock, so signals during GTK startup are buffered too.
    tauri::async_runtime::block_on(async {
        use tokio::signal::unix::{signal, SignalKind};
        Ok(ShutdownSignals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    })
}
fn cleanup_shutdown(socket: &ctl::SocketGuard) {
    screen::shutdown();
    if let Err(error) = socket.cleanup() {
        tracing::warn!(error=%error, "no se pudo limpiar ctl.sock");
    }
}
pub fn run(
    config: Config,
    listener: std::os::unix::net::UnixListener,
    visible: bool,
    socket: Arc<ctl::SocketGuard>,
    mut signals: ShutdownSignals,
) -> Result<()> {
    std::env::set_var("GDK_BACKEND", "x11");
    screen::initialize();
    let state = Arc::new(AppState::new(config)?);
    let exit_socket = socket.clone();
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            ask,
            stop,
            reply_permission,
            new_conversation,
            hide,
            resize,
            needs_screen,
            get_state,
            set_variant,
            set_model,
            open_in_terminal,
            open_url
        ])
        .setup(move |app| {
            let signal_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let signal = tokio::select! {
                    _ = signals.terminate.recv() => "SIGTERM",
                    _ = signals.interrupt.recv() => "SIGINT",
                };
                tracing::info!(signal, "apagado solicitado");
                cleanup_shutdown(&socket);
                signal_handle.exit(0);
            });
            let handle = app.handle().clone();
            let state = handle.state::<Arc<AppState>>().inner().clone();
            let warm = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state.session(&warm).await {
                    tracing::warn!(error=%e,"precalentamiento fallido");
                }
            });
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Ok(cmd) = ctl::receive(stream) {
                        let h = handle.clone();
                        let _ = handle.run_on_main_thread(move || match cmd.cmd.as_str() {
                            "quit" => h.exit(0),
                            "hide" => {
                                let _ = hide(h);
                            }
                            "show" => show(&h, cmd.activation_token.as_deref()),
                            "toggle" => {
                                if window(&h)
                                    .and_then(|w| Ok(w.is_visible()?))
                                    .unwrap_or(false)
                                {
                                    let _ = hide(h);
                                } else {
                                    show(&h, cmd.activation_token.as_deref());
                                }
                            }
                            _ => {}
                        });
                    }
                }
            });
            if visible {
                show(app.handle(), None);
            }
            Ok(())
        })
        .on_page_load(|w, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished
                && std::env::var("QUICKASK_SPIKE").as_deref() == Ok("magenta")
            {
                let _ = w.eval("document.body.style.background = '#FF00FF'");
            }
        })
        .on_window_event(|w, event| {
            let app = w.app_handle();
            match event {
                tauri::WindowEvent::Focused(f) => {
                    tracing::info!(focused = *f, "foco");
                    if *f {
                        let _ = w.center();
                    }
                    if !f {
                        let state = app.state::<Arc<AppState>>();
                        let settled = state
                            .shown
                            .lock()
                            .map(|s| s.is_some_and(|s| s.elapsed().as_millis() > 150))
                            .unwrap_or(false);
                        if state.config.hide_on_blur && settled {
                            let _ = hide(app.clone());
                        }
                    }
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = hide(app.clone());
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())?
        .run(move |_, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                cleanup_shutdown(&exit_socket);
            }
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_web_and_mail_links_open() {
        assert_eq!(
            openable_url(" https://gitlab.example/x "),
            Some("https://gitlab.example/x")
        );
        assert!(openable_url("HTTP://a").is_some());
        assert!(openable_url("mailto:a@b.c").is_some());
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "/home/x",
            "-n",
            "https://a\nb",
        ] {
            assert!(openable_url(bad).is_none(), "{bad}");
        }
    }
    fn state_at(path: &std::path::Path) -> AppState {
        std::fs::write(
            path,
            "# preferencias\nmodel = 'opencode-go/deepseek-v4.1-flash'\nfallback_model = 'opencode/deepseek-v4.1-flash-free'\nvariant = \"\" # esfuerzo\ntheme = 'dark'\n",
        )
        .unwrap();
        AppState::with_config_path(Config::load_path_with(path, |_| None).unwrap(), path.into())
            .unwrap()
    }
    #[test]
    fn fresh_shot_is_preserved_for_first_auto_question_per_invocation() {
        let d = tempfile::tempdir().unwrap();
        let state = state_at(&d.path().join("config.toml"));
        use std::sync::atomic::Ordering::SeqCst;
        assert!(!state.screen_for_question("auto", "Hola"));
        state.fresh_shot.store(true, SeqCst);
        assert!(state.screen_for_question("auto", "Hola"));
        assert!(!state.screen_for_question("auto", "Gracias"));
        assert!(state.screen_for_question("auto", "¿Qué hay en esta pantalla?"));
        state.fresh_shot.store(true, SeqCst);
        assert!(state.screen_for_question("auto", "Hola de nuevo"));
        state.fresh_shot.store(true, SeqCst);
        assert!(!state.screen_for_question("off", "pantalla"));
        assert!(!state.screen_for_question("auto", "Hola"));
        assert!(state.screen_for_question("on", "Hola"));
    }
    #[test]
    fn automatic_capture_still_respects_screen_auto_config() {
        let d = tempfile::tempdir().unwrap();
        let mut state = state_at(&d.path().join("config.toml"));
        state.config.screen_auto = false;
        state
            .fresh_shot
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!state.screen_for_question("auto", "pantalla"));
        assert!(state.screen_for_question("on", "Hola"));
    }
    #[tokio::test]
    async fn variant_without_session_is_saved_and_used_by_new_sessions() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let state = state_at(&p);
        state.change_variant("high").await.unwrap();
        assert_eq!(state.config_snapshot().unwrap().variant, "high");
        assert!(std::fs::read_to_string(&p)
            .unwrap()
            .contains("variant = \"high\" # esfuerzo"));
        assert!(state.change_variant("In valid").await.is_err());
        assert_eq!(state.config_snapshot().unwrap().variant, "high");
        state.change_variant("").await.unwrap();
        assert_eq!(Config::load_path_with(&p, |_| None).unwrap().variant, "");
    }
    #[tokio::test]
    async fn model_change_is_saved_with_the_other_one_as_fallback() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        let state = state_at(&p);
        state
            .change_model("opencode-go/deepseek-v4.1-flash")
            .await
            .unwrap();
        let config = state.config_snapshot().unwrap();
        assert_eq!(config.model, "opencode-go/deepseek-v4.1-flash");
        assert_eq!(config.fallback_model, "opencode/deepseek-v4.1-flash-free");
        assert_eq!(
            crate::opencode::client::session_body(&config).unwrap()["model"],
            serde_json::json!({"providerID":"opencode-go","id":"deepseek-v4.1-flash"})
        );
        let saved = Config::load_path_with(&p, |_| None).unwrap();
        assert_eq!(saved.model, "opencode-go/deepseek-v4.1-flash");
        assert_eq!(saved.fallback_model, "opencode/deepseek-v4.1-flash-free");
        assert!(state.change_model("roto/").await.is_err());
        assert_eq!(
            state.config_snapshot().unwrap().model,
            "opencode-go/deepseek-v4.1-flash"
        );
        state
            .change_model("opencode/deepseek-v4.1-flash-free")
            .await
            .unwrap();
        let saved = Config::load_path_with(&p, |_| None).unwrap();
        assert_eq!(saved.model, "opencode/deepseek-v4.1-flash-free");
        assert_eq!(saved.fallback_model, "opencode-go/deepseek-v4.1-flash");
    }
    #[tokio::test]
    #[ignore = "Usa el servicio OpenCode real y crea una sesión propia de prueba"]
    async fn live_set_variant_changes_next_step_to_high() {
        use futures_util::StreamExt;
        use serde_json::json;
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_ansi(false)
            .try_init();
        let d = tempfile::tempdir().unwrap();
        let state = state_at(&d.path().join("config.toml"));
        let sid = state
            .client
            .create_session(&state.config_snapshot().unwrap())
            .await
            .unwrap();
        state.inner.lock().unwrap().session = Some(sid.clone());
        state.change_variant("high").await.unwrap();
        let response = crate::opencode::sse::subscribe(&state.client)
            .await
            .unwrap();
        state
            .client
            .prompt(&sid, "Responde solo: OK", None)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(90),async {
            let mut stream=response.bytes_stream();let mut decoder=crate::opencode::sse::Decoder::default();let mut high=false;
            while let Some(bytes)=stream.next().await {
                for event in decoder.push(&bytes.unwrap()).unwrap() {
                    if event["data"]["sessionID"]!=sid {continue;}
                    if event["type"]=="session.step.started" {
                        println!("{}",json!({"type":"session.step.started","sessionID":sid,"model":event["data"]["model"]}));
                        assert_eq!(event["data"]["model"]["variant"],"high");high=true;
                    }
                    if event["type"]=="session.execution.succeeded" {assert!(high);return;}
                    assert!(!matches!(event["type"].as_str(),Some("session.execution.failed"|"session.execution.interrupted")));
                }
            }
            panic!("SSE finalizó antes del resultado");
        }).await.unwrap();
        assert_eq!(
            Config::load_path(&state.config_path).unwrap().variant,
            "high"
        );
        state.change_variant("").await.unwrap();
        let session = state.client.session(&sid).await.unwrap();
        println!(
            "{}",
            json!({"requestedVariant":"","storedModel":session["data"]["model"]})
        );
        assert!(matches!(
            session["data"]["model"]["variant"].as_str(),
            Some("default") | None
        ));
        assert_eq!(Config::load_path(&state.config_path).unwrap().variant, "");
    }
}
