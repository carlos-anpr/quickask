use crate::watchdog::{summary_prompt, Limits, Watchdog};
use crate::{
    config::{self, Config},
    events::UiEvent,
    opencode::{client::Client, sse},
    translate::TurnCtx,
};
use anyhow::Result;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::sync::watch;
/// Relevo al modelo de respaldo: solo si el turno falló sin haber pintado texto,
/// o si el error es de los que se resuelven cambiando de modelo.
pub fn should_fallback(outcome: &str, emitted_text: bool, error: &str) -> bool {
    !emitted_text && (outcome == "failed" || provider_error(error))
}
fn provider_error(error: &str) -> bool {
    let error = error.to_lowercase();
    [
        "rate limit",
        "429",
        "500",
        "502",
        "503",
        "504",
        "5xx",
        "overload",
        "quota",
        "cuota",
        "too many requests",
        "unavailable",
    ]
    .iter()
    .any(|pattern| error.contains(pattern))
}
/// Datos de un turno: la pregunta, su captura y el id que comparten todos sus eventos.
pub struct Turn<'a> {
    pub session: &'a str,
    pub id: &'a str,
    pub text: &'a str,
    pub file: Option<&'a Path>,
}
/// Contexto limpio por intento: historial previo y estado de reposo de la sesión.
async fn prepare(client: &Client, config: &Config, turn: &Turn<'_>) -> Result<TurnCtx> {
    let data = client.session(turn.session).await?;
    let (_, id) = config.model_parts()?;
    // Un relevo anterior dejó la sesión en el modelo de respaldo: este turno vuelve al principal.
    if data["data"]["model"]["id"]
        .as_str()
        .is_some_and(|m| m != id)
    {
        client
            .switch_model(turn.session, config, &config.variant)
            .await?;
    }
    let mut ctx = TurnCtx {
        session_id: turn.session.into(),
        turn_id: turn.id.into(),
        ..Default::default()
    };
    if let Some(items) = client.list_messages(turn.session).await?["data"].as_array() {
        for m in items {
            if let Some(id) = m["id"].as_str() {
                ctx.prior_messages.insert(id.into());
            }
        }
    }
    ctx.prior_idle = data["data"]["time"]["idle"].clone();
    Ok(ctx)
}
pub async fn execute(
    client: &Client,
    config: &Config,
    turn: &Turn<'_>,
    cancel: &mut watch::Receiver<bool>,
    emit: &(dyn Fn(UiEvent) + Send + Sync),
) -> Result<(bool, String)> {
    use std::sync::{atomic::Ordering::Relaxed, Arc, Mutex};
    let mut fallback_used = false;
    // Modelo de este intento: el principal y, tras un relevo, el de respaldo.
    let mut attempt = config.clone();
    // Tras un corte de seguridad se pide un resumen breve en el mismo turno.
    let mut prompt_text = turn.text.to_owned();
    let mut prompt_file = turn.file;
    let mut limits = Limits::from_config(config);
    let mut tripped = false;
    loop {
        let mut ctx = prepare(client, &attempt, turn).await?;
        if *cancel.borrow() {
            emit(UiEvent::Done {
                turn_id: turn.id.into(),
                outcome: "interrupted".into(),
                error: None,
            });
            return Ok((false, String::new()));
        }
        let response = sse::subscribe(client).await?;
        client
            .prompt(turn.session, &prompt_text, prompt_file)
            .await?;
        tracing::info!(turn_id = turn.id, "prompt enviado");
        let can_fallback = !fallback_used
            && !tripped
            && !config.fallback_model.is_empty()
            && config.fallback_model != config.model;
        let shown = std::sync::atomic::AtomicBool::new(false);
        let retry = std::sync::atomic::AtomicBool::new(false);
        let done = Mutex::new(None::<(String, String)>);
        // Vigilante: corta si se atasca, repite un paso o se eterniza.
        let dog = Arc::new(Mutex::new(Watchdog::new(limits, Instant::now())));
        let reason = Arc::new(Mutex::new(None::<String>));
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let timer = {
            let (dog, reason, stop_tx, mut external) =
                (dog.clone(), reason.clone(), stop_tx.clone(), cancel.clone());
            tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(1));
                loop {
                    tokio::select! {
                        changed = external.changed() => {
                            if changed.is_err() || *external.borrow() {
                                let _ = stop_tx.send(true);
                                return;
                            }
                        }
                        _ = every.tick() => {
                            let why = dog.lock().ok().and_then(|d| d.tick(Instant::now()));
                            if let Some(why) = why {
                                if let Ok(mut r) = reason.lock() {
                                    r.get_or_insert(why);
                                }
                                let _ = stop_tx.send(true);
                                return;
                            }
                        }
                    }
                }
            })
        };
        let sink = |e: UiEvent| {
            let why = dog
                .lock()
                .ok()
                .and_then(|mut d| d.observe(&e, Instant::now()));
            if let Some(why) = why {
                if let Ok(mut r) = reason.lock() {
                    r.get_or_insert(why);
                }
                let _ = stop_tx.send(true);
            }
            match &e {
                UiEvent::Text { .. } | UiEvent::TextFinal { .. } => {
                    shown.store(true, Relaxed);
                }
                UiEvent::Done { outcome, error, .. } => {
                    // Corte de seguridad: este `done` lo sustituye el resumen.
                    if reason.lock().is_ok_and(|r| r.is_some()) {
                        return;
                    }
                    let error = error.clone().unwrap_or_default();
                    if can_fallback && should_fallback(outcome, shown.load(Relaxed), &error) {
                        // Este `done` lo sustituye el relevo: el turno termina con el segundo intento.
                        retry.store(true, Relaxed);
                        return;
                    }
                    if let Ok(mut d) = done.lock() {
                        *d = Some((outcome.clone(), error));
                    }
                }
                _ => {}
            }
            emit(e);
        };
        let consumed = sse::consume(client, response, &mut ctx, &mut stop_rx, &sink).await;
        timer.abort();
        consumed?;
        let why = reason.lock().ok().and_then(|r| r.clone());
        if let Some(why) = why {
            tracing::warn!(turn_id = turn.id, reason = %why, "turno cortado por seguridad");
            if let Err(e) = client.interrupt(turn.session).await {
                tracing::warn!(error=%e, "no se pudo interrumpir tras el corte");
            }
            if tripped {
                emit(UiEvent::Done {
                    turn_id: turn.id.into(),
                    outcome: "interrupted".into(),
                    error: Some(format!("Detenido por seguridad: {why}")),
                });
                return Ok((false, ctx.text));
            }
            emit(UiEvent::Notice {
                turn_id: turn.id.into(),
                text: format!(
                    "Detenido por seguridad: {why}. Pidiendo un resumen de lo conseguido…"
                ),
            });
            tripped = true;
            prompt_text = summary_prompt(&why);
            prompt_file = None;
            limits = Limits::summary();
            // Dejar que OpenCode cierre la ejecución interrumpida antes de reenviar.
            tokio::time::sleep(Duration::from_millis(800)).await;
            continue;
        }
        let outcome = done.lock().ok().and_then(|d| d.clone());
        if !retry.load(Relaxed) {
            return Ok((
                outcome.as_ref().is_some_and(|(o, _)| o == "succeeded"),
                ctx.text,
            ));
        }
        fallback_used = true;
        emit(UiEvent::Notice {
            turn_id: turn.id.into(),
            text: format!(
                "{} no disponible; reintentando con {}",
                config::model_label(&config.model),
                config::model_label(&config.fallback_model)
            ),
        });
        attempt.model = config.fallback_model.clone();
        tracing::info!(fallback_model=%config.fallback_model,"relevo al modelo de respaldo");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_only_without_text_and_after_a_failure() {
        for (outcome, text, error, expected) in [
            ("failed", false, "", true),
            (
                "failed",
                false,
                "El agente no pudo completar la respuesta",
                true,
            ),
            ("failed", false, "429 Too Many Requests", true),
            ("failed", false, "rate limit exceeded", true),
            ("failed", false, "503 Service Unavailable", true),
            ("failed", false, "overloaded", true),
            ("failed", false, "cuota agotada", true),
            ("failed", true, "429 Too Many Requests", false),
            ("succeeded", false, "", false),
            ("succeeded", true, "rate limit", false),
            ("interrupted", false, "", false),
            ("", false, "", false),
        ] {
            assert_eq!(
                should_fallback(outcome, text, error),
                expected,
                "{outcome}/{text}/{error}"
            );
        }
        // Error de proveedor en el SSE aunque el turno no se marque como fallido.
        assert!(should_fallback("succeeded", false, "500 internal error"));
    }
}
