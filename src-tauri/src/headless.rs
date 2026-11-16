use crate::{config::Config, events::UiEvent, opencode::client::Client, screen};
use anyhow::Result;
pub async fn run(text: String, with_screen: bool) -> Result<bool> {
    let started = std::time::Instant::now();
    let first = std::sync::atomic::AtomicBool::new(true);
    let config = Config::load()?;
    let client = Client::new()?;
    let turn = uuid::Uuid::new_v4().to_string();
    let emit = |event: UiEvent| {
        if matches!(event, UiEvent::Text { .. })
            && first.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            tracing::info!(ms = started.elapsed().as_millis(), "primer delta desde ask");
        }
        if matches!(event, UiEvent::Done { .. }) {
            tracing::info!(ms = started.elapsed().as_millis(), "done desde ask");
        }
        if let Ok(line) = serde_json::to_string(&event) {
            println!("{line}");
        }
    };
    emit(UiEvent::Backend {
        status: "connecting".into(),
        detail: None,
    });
    let session = match client.create_session(&config).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error=%e,"creación de sesión fallida");
            emit(UiEvent::Done {
                turn_id: turn,
                outcome: "failed".into(),
                error: Some("No se pudo conectar con OpenCode".into()),
            });
            return Ok(false);
        }
    };
    emit(UiEvent::Backend {
        status: "ready".into(),
        detail: None,
    });
    emit(UiEvent::TurnStarted {
        turn_id: turn.clone(),
        text: text.clone(),
        screen: with_screen,
    });
    let file = if with_screen {
        emit(UiEvent::Screen {
            turn_id: turn.clone(),
            status: "capturing".into(),
            thumb: None,
            error: None,
        });
        match screen::capture().await {
            Ok(p) => {
                let thumb = screen::thumbnail(&p).ok();
                emit(UiEvent::Screen {
                    turn_id: turn.clone(),
                    status: "attached".into(),
                    thumb,
                    error: None,
                });
                Some(p)
            }
            Err(e) => {
                tracing::warn!(error=%e,"captura fallida");
                emit(UiEvent::Screen {
                    turn_id: turn.clone(),
                    status: "failed".into(),
                    thumb: None,
                    error: Some("No se pudo capturar la pantalla".into()),
                });
                None
            }
        }
    } else {
        None
    };
    let (_tx, mut rx) = tokio::sync::watch::channel(false);
    let _ = client.rename(&session, &text).await;
    let result = crate::engine::execute(
        &client,
        &config,
        &crate::engine::Turn {
            session: &session,
            id: &turn,
            text: &text,
            file: file.as_deref(),
        },
        &mut rx,
        &emit,
    )
    .await;
    if let Some(p) = file {
        let _ = tokio::fs::remove_file(p).await;
    }
    match result {
        Ok((ok, _)) => Ok(ok),
        Err(e) => {
            tracing::error!(error=%e,"turno fallido");
            emit(UiEvent::Done {
                turn_id: turn,
                outcome: "failed".into(),
                error: Some("No se pudo completar la respuesta".into()),
            });
            Ok(false)
        }
    }
}
