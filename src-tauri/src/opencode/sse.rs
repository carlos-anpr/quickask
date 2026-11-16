use super::client::Client;
use crate::{
    events::UiEvent,
    translate::{translate, TurnCtx},
};
use anyhow::{ensure, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::watch;
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    data: Vec<String>,
}
impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.buffer.extend_from_slice(bytes);
        ensure!(
            self.buffer.len() < 8 * 1024 * 1024,
            "Evento SSE demasiado grande"
        );
        let mut events = vec![];
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line)?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let text = self.data.join("\n");
                    self.data.clear();
                    events.push(serde_json::from_str(&text)?);
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
        }
        Ok(events)
    }
}
pub async fn subscribe(client: &Client) -> Result<reqwest::Response> {
    client
        .request(reqwest::Method::GET, "/api/event", None, true)
        .await
}
pub async fn consume(
    client: &Client,
    initial: reqwest::Response,
    ctx: &mut TurnCtx,
    cancel: &mut watch::Receiver<bool>,
    emit: &(dyn Fn(UiEvent) + Send + Sync),
) -> Result<()> {
    let started = Instant::now();
    let mut first = true;
    let mut response = initial;
    let mut backoff = Duration::from_millis(250);
    let mut reconnected = false;
    loop {
        let mut stream = response.bytes_stream();
        let mut decoder = Decoder::default();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        ctx.done = true;
                        emit(UiEvent::Done { turn_id: ctx.turn_id.clone(), outcome: "interrupted".into(), error: None });
                        return Ok(());
                    }
                },
                _ = heartbeat.tick() => {
                    if let Err(e) = reconcile(client, ctx, emit, false).await {
                        tracing::warn!(error=%e, "reconciliación pendiente");
                    }
                    if ctx.done { return Ok(()); }
                },
                part = stream.next() => {
                    match part {
                        Some(Ok(bytes)) => {
                            for event in decoder.push(&bytes)? {
                                for translated in translate(&event, ctx) {
                                    if matches!(translated, UiEvent::Text { .. }) && first {
                                        first = false;
                                        tracing::info!(ms=started.elapsed().as_millis(), "primer delta desde prompt");
                                    }
                                    if matches!(translated, UiEvent::Done { .. }) && reconnected {
                                        if let Ok(messages) = client.list_messages(&ctx.session_id).await {
                                            restore_messages(&messages, ctx, emit);
                                        }
                                    }
                                    emit(translated);
                                }
                                if ctx.done {
                                    tracing::info!(ms=started.elapsed().as_millis(), "done");
                                    return Ok(());
                                }
                            }
                        },
                        _ => break,
                    }
                }
            }
        }
        loop {
            tracing::warn!(backoff_ms = backoff.as_millis(), "reconexión SSE");
            emit(UiEvent::Backend {
                status: "connecting".into(),
                detail: None,
            });
            tokio::select! {_=tokio::time::sleep(backoff)=>{},_=cancel.changed()=>{ctx.done=true;emit(UiEvent::Done{turn_id:ctx.turn_id.clone(),outcome:"interrupted".into(),error:None});return Ok(());}}
            match subscribe(client).await {
                Ok(r) => {
                    response = r;
                    reconnected = true;
                    emit(UiEvent::Backend {
                        status: "ready".into(),
                        detail: None,
                    });
                    if let Err(e) = reconcile(client, ctx, emit, true).await {
                        tracing::warn!(error=%e,"reconciliación pendiente");
                    }
                    backoff = Duration::from_millis(250);
                    break;
                }
                Err(_) => {
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            }
        }
        if ctx.done {
            return Ok(());
        }
    }
}
fn restore_messages(messages: &Value, ctx: &mut TurnCtx, emit: &(dyn Fn(UiEvent) + Send + Sync)) {
    let mut text = String::new();
    let mut usage_restored = false;
    if let Some(items) = messages["data"].as_array() {
        for msg in items.iter().rev() {
            let id = msg["id"].as_str().unwrap_or_default();
            if msg["type"] != "assistant" || ctx.prior_messages.contains(id) {
                continue;
            }
            if ctx.usage.step(id, msg) {
                usage_restored = true;
                ctx.current_step = Some(id.into());
            }
            if let Some(content) = msg["content"].as_array() {
                // OpenCode ordinals count text segments, independently from reasoning/tools.
                for (i, part) in content.iter().filter(|p| p["type"] == "text").enumerate() {
                    let segment = part["text"].as_str().unwrap_or_default();
                    text.push_str(segment);
                    emit(UiEvent::TextFinal {
                        turn_id: ctx.turn_id.clone(),
                        message_id: id.into(),
                        ordinal: i as u64,
                        text: segment.into(),
                    });
                }
            }
        }
    }
    ctx.text = text;
    if usage_restored {
        ctx.usage.reconciled();
        emit(ctx.usage.event(&ctx.turn_id));
    }
}
fn completed<'a>(session: &'a Value, ctx: &TurnCtx) -> Option<&'a str> {
    let s = &session["data"];
    if s["time"]["idle"].is_null() || s["time"]["idle"] == ctx.prior_idle {
        return None;
    }
    s["outcome"]
        .as_str()
        .filter(|o| ["succeeded", "failed", "interrupted"].contains(o))
}
pub async fn reconcile(
    client: &Client,
    ctx: &mut TurnCtx,
    emit: &(dyn Fn(UiEvent) + Send + Sync),
    force_snapshot: bool,
) -> Result<()> {
    let session = client.session(&ctx.session_id).await?;
    let outcome = completed(&session, ctx).map(str::to_owned);
    if force_snapshot || outcome.is_some() {
        restore_messages(&client.list_messages(&ctx.session_id).await?, ctx, emit);
    }
    let pending = client.list_permissions(&ctx.session_id).await?;
    let mut current = std::collections::HashSet::new();
    if let Some(items) = pending["data"].as_array() {
        for p in items {
            let id = p["id"].as_str().unwrap_or_default().to_owned();
            current.insert(id.clone());
            if !ctx.permissions.contains(&id) {
                let ev = json!({"type":"permission.asked","data":p});
                for e in translate(&ev, ctx) {
                    emit(e);
                }
            }
        }
    }
    for closed in ctx.permissions.difference(&current) {
        emit(UiEvent::PermissionClosed {
            request_id: closed.clone(),
        });
    }
    ctx.permissions = current;
    if let Some(outcome) = outcome {
        ctx.done = true;
        emit(UiEvent::Done {
            turn_id: ctx.turn_id.clone(),
            outcome,
            error: None,
        });
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_crlf_comments() {
        let data = ": heartbeat\r\ndata: {\"type\":\"prueba\",\r\ndata: \"text\":\"á\"}\r\n\r\n"
            .as_bytes();
        let mut d = Decoder::default();
        let mut got = vec![];
        for b in data {
            got.extend(d.push(&[*b]).unwrap());
        }
        assert_eq!(got, vec![json!({"type":"prueba","text":"á"})]);
    }
    #[test]
    fn multiple_events_and_partial() {
        let mut d = Decoder::default();
        assert!(d.push(b"data: {\"x\"").unwrap().is_empty());
        assert_eq!(
            d.push(b":1}\n\ndata: {\"x\":2}\n\n:ping\n\n").unwrap(),
            vec![json!({"x":1}), json!({"x":2})]
        );
    }
    #[test]
    fn restore_only_current_text_with_text_ordinals() {
        let mut ctx = TurnCtx {
            prior_messages: ["old".to_owned()].into(),
            ..Default::default()
        };
        let messages = json!({"data":[
            {"id":"new","type":"assistant","content":[{"type":"reasoning","text":"private"},{"type":"tool"},{"type":"text","text":"Hola"},{"type":"text","text":" mundo"}]},
            {"id":"user","type":"user","content":[]},
            {"id":"old","type":"assistant","content":[{"type":"text","text":"anterior"}]}
        ]});
        let events = std::sync::Mutex::new(Vec::new());
        restore_messages(&messages, &mut ctx, &|e| events.lock().unwrap().push(e));
        let es = events.into_inner().unwrap();
        assert_eq!(ctx.text, "Hola mundo");
        assert!(matches!(&es[0], UiEvent::TextFinal { ordinal: 0, text, .. } if text == "Hola"));
        assert!(matches!(&es[1], UiEvent::TextFinal { ordinal: 1, .. }));
    }
    #[test]
    fn previous_outcome_does_not_finish_new_turn() {
        let ctx = TurnCtx {
            prior_idle: json!(123),
            ..Default::default()
        };
        assert!(completed(
            &json!({"data":{"outcome":"succeeded","time":{"idle":123}}}),
            &ctx
        )
        .is_none());
        assert!(completed(&json!({"data":{"outcome":"succeeded","time":{}}}), &ctx).is_none());
        assert_eq!(
            completed(
                &json!({"data":{"outcome":"succeeded","time":{"idle":456}}}),
                &ctx
            ),
            Some("succeeded")
        );
    }
    #[tokio::test]
    async fn reconnect_completed_turn_keeps_done_as_last_event() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::for_test(format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            for _ in 0..5 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0u8; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let body = if path == "/api/event" {
                    String::new()
                } else if path.contains("/message?") {
                    json!({"data":[{"id":"msg_new","type":"assistant","content":[{"type":"text","text":"recuperado"}]}]}).to_string()
                } else if path.ends_with("/permission") {
                    json!({"data":[]}).to_string()
                } else {
                    json!({"data":{"outcome":"succeeded","time":{"idle":456}}}).to_string()
                };
                let content_type = if path == "/api/event" {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let response = subscribe(&client).await.unwrap();
        let mut ctx = TurnCtx {
            session_id: "ses_test".into(),
            turn_id: "turn".into(),
            prior_idle: json!(123),
            ..Default::default()
        };
        let (_tx, mut rx) = watch::channel(false);
        let events = std::sync::Mutex::new(Vec::new());
        tokio::time::timeout(
            Duration::from_secs(3),
            consume(&client, response, &mut ctx, &mut rx, &|e| {
                events.lock().unwrap().push(e)
            }),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        let events = events.into_inner().unwrap();
        assert!(ctx.done);
        assert!(
            matches!(events.last(), Some(UiEvent::Done { outcome, .. }) if outcome == "succeeded")
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, UiEvent::TextFinal { text, .. } if text == "recuperado")));
    }
    #[test]
    fn usage_reconciliation_restores_only_current_turn_and_never_duplicates_steps() {
        let mut ctx = TurnCtx {
            prior_messages: ["old".into()].into(),
            ..Default::default()
        };
        let first =
            json!({"cost":0.1,"tokens":{"input":10,"output":3,"reasoning":2,"cache":{"read":4}}});
        ctx.usage.update(None, &first);
        ctx.usage.step("new1", &first);
        let messages = json!({"data":[
            {"id":"new2","type":"assistant","cost":0.2,"tokens":{"input":20,"output":5,"reasoning":6,"cache":{"read":8}}},
            {"id":"new1","type":"assistant","cost":0.1,"tokens":{"input":10,"output":3,"reasoning":2,"cache":{"read":4}}},
            {"id":"old","type":"assistant","cost":100,"tokens":{"input":999,"output":999}}
        ]});
        restore_messages(&messages, &mut ctx, &|_| {});
        restore_messages(&messages, &mut ctx, &|_| {});
        ctx.usage.step("new1", &first);
        let UiEvent::Usage {
            cost,
            tokens_in,
            tokens_out,
            tokens_reasoning,
            tokens_cache,
            ..
        } = ctx.usage.event("")
        else {
            panic!();
        };
        assert!((cost - 0.3).abs() < 1e-12);
        assert_eq!(
            (tokens_in, tokens_out, tokens_reasoning, tokens_cache),
            (30, 8, 8, 12)
        );
    }
}
