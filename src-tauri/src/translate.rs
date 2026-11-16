use crate::events::UiEvent;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[derive(Default)]
pub struct TurnCtx {
    pub session_id: String,
    pub turn_id: String,
    pub done: bool,
    pub seen: HashSet<String>,
    pub tools: HashMap<String, String>,
    pub titles: HashMap<String, String>,
    pub messages: HashSet<String>,
    pub prior_messages: HashSet<String>,
    pub permissions: HashSet<String>,
    pub text: String,
    pub prior_idle: Value,
    pub usage: crate::usage::TurnUsage,
    pub current_step: Option<String>,
}
pub fn string(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_owned()
}
pub fn translate(ev: &Value, ctx: &mut TurnCtx) -> Vec<UiEvent> {
    let d = &ev["data"];
    if d["sessionID"].as_str() != Some(&ctx.session_id) || ctx.done {
        return vec![];
    }
    if let Some(id) = ev["id"].as_str() {
        if !ctx.seen.insert(id.into()) {
            return vec![];
        }
    }
    let typ = ev["type"].as_str().unwrap_or_default();
    let t = ctx.turn_id.clone();
    let mid = string(d, "assistantMessageID");
    let ordinal = d["ordinal"].as_u64().unwrap_or(0);
    let event = match typ {
        "session.text.delta" => {
            ctx.messages.insert(mid.clone());
            let delta = string(d, "delta");
            ctx.text.push_str(&delta);
            UiEvent::Text {
                turn_id: t,
                message_id: mid,
                ordinal,
                delta,
            }
        }
        "session.text.ended" => {
            ctx.messages.insert(mid.clone());
            UiEvent::TextFinal {
                turn_id: t,
                message_id: mid,
                ordinal,
                text: string(d, "text"),
            }
        }
        "session.reasoning.delta" => UiEvent::Reasoning {
            turn_id: t,
            message_id: mid,
            delta: string(d, "delta"),
        },
        "permission.asked" => {
            let id = string(d, "id");
            ctx.permissions.insert(id.clone());
            UiEvent::Permission {
                turn_id: t,
                request_id: id,
                action: string(d, "action"),
                resources: d["resources"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        }
        "permission.replied" => {
            let id = d["requestID"]
                .as_str()
                .or(d["id"].as_str())
                .unwrap_or_default()
                .to_owned();
            ctx.permissions.remove(&id);
            UiEvent::PermissionClosed { request_id: id }
        }
        "session.step.started" => {
            ctx.current_step = d["assistantMessageID"].as_str().map(str::to_owned);
            return vec![];
        }
        "session.step.ended" => {
            if mid.is_empty() || !ctx.usage.step(&mid, d) {
                return vec![];
            }
            ctx.current_step = Some(mid);
            ctx.usage.event(&t)
        }
        "session.usage.updated" => {
            ctx.usage.update(ctx.current_step.as_deref(), d);
            ctx.usage.event(&t)
        }
        s if s.starts_with("session.tool.") => {
            let id = string(d, "id");
            if let Some(n) = d["name"].as_str() {
                ctx.tools.insert(id.clone(), n.into());
            }
            let status = if s.contains("error") || s.contains("fail") {
                "error"
            } else {
                match s {
                    "session.tool.input.started" => "pending",
                    "session.tool.called" | "session.tool.progress" => "running",
                    "session.tool.success" => "ok",
                    _ => return vec![],
                }
            };
            let name = ctx
                .tools
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "herramienta".into());
            // Only `input.started`/`called` carry `input`: later events keep the last title.
            let title = tool_title(&name, &d["input"]).or_else(|| ctx.titles.get(&id).cloned());
            if let Some(t) = &title {
                ctx.titles.insert(id.clone(), t.clone());
            }
            // `execute` (Code Mode) only reveals the real calls once it has run:
            // toolCalls in metadata names each MCP tool with its input.
            let improved = (name == "execute" && status == "ok")
                .then(|| tool_calls_title(&d["metadata"]["toolCalls"]))
                .flatten()
                .filter(|better| {
                    title
                        .as_deref()
                        .is_none_or(|current| !describes_same_calls(current, better))
                });
            if let Some(better) = &improved {
                ctx.titles.insert(id.clone(), better.clone());
            }
            let mut events = vec![UiEvent::Tool {
                turn_id: t.clone(),
                call_id: id.clone(),
                name: name.clone(),
                status: status.into(),
                title,
            }];
            if let Some(better) = improved {
                events.push(UiEvent::Tool {
                    turn_id: t,
                    call_id: id,
                    name,
                    status: "ok".into(),
                    title: Some(better),
                });
            }
            return events;
        }
        s if s.starts_with("session.execution.") && s != "session.execution.started" => {
            ctx.done = true;
            UiEvent::Done {
                turn_id: t,
                outcome: if s.ends_with("succeeded") {
                    "succeeded"
                } else if s.contains("interrupt") || s.contains("cancel") {
                    "interrupted"
                } else {
                    "failed"
                }
                .into(),
                error: if s.contains("fail") {
                    Some("El agente no pudo completar la respuesta".into())
                } else {
                    None
                },
            }
        }
        _ => return vec![],
    };
    vec![event]
}
// Parse just enough shell syntax to remove setup prefixes from a display title.
// The command itself is never modified or executed here.
fn shell_word(input: &str) -> Option<(&str, &str)> {
    shell_token(input, true)
}
fn shell_token(input: &str, operators: bool) -> Option<(&str, &str)> {
    let input = input.trim_start();
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() || (operators && matches!(ch, '&' | ';' | '|')) {
            return (index > 0).then_some((&input[..index], &input[index..]));
        }
    }
    (!input.is_empty()).then_some((input, ""))
}
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn strip_command_setup(mut command: &str) -> &str {
    loop {
        command = command.trim_start();
        let Some((first, rest)) = shell_word(command) else {
            return command;
        };
        if first == "cd" || first == "source" {
            let Some((_, after_path)) = shell_word(rest) else {
                return command;
            };
            if let Some(after_and) = after_path.trim_start().strip_prefix("&&") {
                command = after_and;
                continue;
            }
            return command;
        }
        let mut remaining = if first == "export" { rest } else { command };
        let mut assignments = 0;
        while let Some((word, rest)) = shell_word(remaining) {
            if !is_assignment(word) {
                break;
            }
            assignments += 1;
            remaining = rest;
        }
        if assignments == 0 {
            return command;
        }
        remaining = remaining.trim_start();
        command = remaining
            .strip_prefix("&&")
            .or_else(|| remaining.strip_prefix(';'))
            .unwrap_or(remaining);
    }
}
fn command_parts(command: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut chars = command.char_indices().peekable();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    while let Some((index, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if matches!(ch, '|' | ';' | '\n')
            || (ch == '&' && chars.peek().is_some_and(|(_, c)| *c == '&'))
        {
            // A redirection like 2>&1 is not a command boundary.
            parts.push(&command[start..index]);
            start = index + ch.len_utf8();
            if matches!(ch, '&' | '|') && chars.peek().is_some_and(|(_, c)| *c == ch) {
                chars.next();
                start += 1;
            }
        }
    }
    parts.push(&command[start..]);
    parts
}
fn playwright_title(command: &str) -> Option<String> {
    let (binary, args) = shell_word(strip_command_setup(command))?;
    let (binary, mut args) = if binary == "npx" {
        shell_word(args)?
    } else {
        (binary, args)
    };
    let binary = binary.trim_matches(['\'', '"']);
    if std::path::Path::new(binary).file_name()?.to_str()? != "playwright-cli" {
        return None;
    }
    let mut words = vec!["playwright-cli"];
    while let Some((word, rest)) = shell_token(args, false) {
        if !word.starts_with("-s=") {
            words.push(word);
        }
        args = rest;
    }
    Some(words.join(" "))
}
fn command_title(command: &str) -> String {
    let joined = command.replace("\\\r\n", " ").replace("\\\n", " ");
    let command = strip_command_setup(&joined);
    command_parts(command)
        .into_iter()
        .find_map(playwright_title)
        .unwrap_or_else(|| command.to_owned())
}
fn tool_title(name: &str, v: &Value) -> Option<String> {
    let raw = if name == "execute" {
        v["code"]
            .as_str()
            .and_then(execute_title)
            .unwrap_or_else(|| generic_title(v))
    } else if let Some(command) = v["command"].as_str() {
        command_title(command)
    } else {
        generic_title(v)
    };
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.chars().take(80).collect())
    }
}
fn generic_title(v: &Value) -> String {
    if let Some(agent) = v["agent"].as_str() {
        return format!(
            "{}: {}",
            agent,
            v["description"].as_str().unwrap_or_default()
        );
    }
    [
        "name",
        "skill",
        "url",
        "filePath",
        "path",
        "query",
        "ticket_id",
        "id",
        "project",
    ]
    .iter()
    .find_map(|key| match &v[*key] {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
    .unwrap_or_default()
}
// ---- títulos de `execute` (Code Mode): tools["srv"].método({ args }) ----
struct CodeCall {
    server: String,
    method: String,
    args: String,
}
static CODE_CALL: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"\btools(?:\[\s*["']([^"'\\]+)["']\s*\]|\.([A-Za-z_][\w-]*))\s*(?:\.([A-Za-z_]\w*)|\[\s*["']([^"'\\]+)["']\s*\])\s*\(\s*(\{[^{}]*\})?"#,
    )
    .ok()
});
fn execute_calls(code: &str) -> Vec<CodeCall> {
    let Some(re) = CODE_CALL.as_ref() else {
        return Vec::new();
    };
    re.captures_iter(code)
        .filter_map(|c| {
            let server = c.get(1).or(c.get(2))?.as_str().to_owned();
            let method = c.get(3).or(c.get(4))?.as_str().to_owned();
            let args = c.get(5).map(|m| m.as_str().to_owned()).unwrap_or_default();
            Some(CodeCall {
                server,
                method,
                args,
            })
        })
        .collect()
}
// `{ ticket_id: 12345 }` → `ticket_id: 12345` (sin llaves, comillas ni espacios de más).
fn args_title(args: &str) -> String {
    let inner = args.trim().trim_start_matches('{').trim_end_matches('}');
    compact(&inner.replace(['"', '\'', '`'], ""))
}
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn first_code_line(code: &str) -> Option<String> {
    code.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("/*")
                && !line.starts_with('*')
        })
        .map(compact)
        .filter(|line| !line.is_empty())
}
// "freshservice get_ticket ticket_id: 12345" (varias llamadas unidas por " · ").
fn execute_title(code: &str) -> Option<String> {
    let mut labels = Vec::new();
    for call in execute_calls(code) {
        let server = call.server.strip_suffix("-mcp").unwrap_or(&call.server);
        let args = args_title(&call.args);
        let label = if args.is_empty() {
            format!("{server} {}", call.method)
        } else {
            format!("{server} {} {args}", call.method)
        };
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    if labels.is_empty() {
        first_code_line(code)
    } else {
        Some(labels.join(" · "))
    }
}
// "freshservice-mcp.get_ticket ticket_id: 12345" a partir de metadata.toolCalls.
fn tool_calls_title(calls: &Value) -> Option<String> {
    let mut labels = Vec::new();
    for call in calls.as_array()? {
        let tool = call["tool"].as_str().unwrap_or_default().trim();
        if tool.is_empty() {
            continue;
        }
        let args = value_title(&call["input"]);
        let label = if args.is_empty() {
            tool.to_owned()
        } else {
            format!("{tool} {args}")
        };
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    (!labels.is_empty()).then(|| labels.join(" · "))
}
fn value_title(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let value = scalar_title(value);
                if value.is_empty() {
                    key.clone()
                } else {
                    format!("{key}: {value}")
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Null => String::new(),
        other => scalar_title(other),
    }
}
fn scalar_title(value: &Value) -> String {
    match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
// The code scan and metadata.toolCalls name the same call differently
// ("freshservice get_ticket" vs "freshservice-mcp.get_ticket"): do not re-emit
// the chip when both titles describe the same calls.
fn describes_same_calls(current: &str, candidate: &str) -> bool {
    fn normalized(title: &str) -> String {
        title
            .to_lowercase()
            .replace("-mcp", "")
            .replace('.', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    normalized(current) == normalized(candidate)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_replay() {
        for fixture in [
            include_str!("../../fixtures/sse-turno-con-permiso-shell.jsonl"),
            include_str!("../../fixtures/sse-turno-con-subagente-y-vision.jsonl"),
        ] {
            let events: Vec<Value> = fixture
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            let mut c = TurnCtx {
                session_id: string(&events[0]["data"], "sessionID"),
                turn_id: "turn".into(),
                ..Default::default()
            };
            let mut tool = false;
            let mut permission = false;
            let mut accumulated = HashMap::<(String, u64), String>::new();
            let mut ended = 0;
            let mut done = false;
            for ev in &events {
                for e in translate(ev, &mut c) {
                    match e {
                        UiEvent::Text {
                            message_id,
                            ordinal,
                            delta,
                            ..
                        } => accumulated
                            .entry((message_id, ordinal))
                            .or_default()
                            .push_str(&delta),
                        UiEvent::TextFinal {
                            message_id,
                            ordinal,
                            text,
                            ..
                        } => {
                            assert_eq!(accumulated.get(&(message_id, ordinal)).unwrap(), &text);
                            ended += 1;
                        }
                        UiEvent::Tool { status, .. } => {
                            tool = true;
                            assert!(
                                ["pending", "running", "ok", "error"].contains(&status.as_str())
                            );
                        }
                        UiEvent::Permission { .. } => permission = true,
                        UiEvent::Done { outcome, .. } => {
                            assert_eq!(outcome, "succeeded");
                            done = true;
                        }
                        _ => {}
                    }
                }
            }
            assert!(tool && done && ended > 0);
            assert_eq!(permission, fixture.contains("permission.asked"));
            assert!(c.permissions.is_empty());
        }
    }
    #[test]
    fn foreign_and_duplicate() {
        let mut c = TurnCtx {
            session_id: "ours".into(),
            ..Default::default()
        };
        let mut e = serde_json::json!({"id":"evt1","type":"session.text.delta","data":{"sessionID":"other","delta":"x"}});
        assert!(translate(&e, &mut c).is_empty());
        e["data"]["sessionID"] = "ours".into();
        assert_eq!(translate(&e, &mut c).len(), 1);
        assert!(translate(&e, &mut c).is_empty());
    }
    #[test]
    fn title_limit() {
        assert_eq!(
            tool_title("shell", &serde_json::json!({"command":"á".repeat(90)}))
                .unwrap()
                .chars()
                .count(),
            80
        );
    }
    #[test]
    fn tool_titles_show_skills_systems_and_browser_actions() {
        for (input, expected) in [
            (serde_json::json!({"name":"freshservice"}), "freshservice"),
            (serde_json::json!({"skill":"gitlab"}), "gitlab"),
            (
                serde_json::json!({"url":"https://example.org"}),
                "https://example.org",
            ),
            (serde_json::json!({"ticket_id":12345}), "12345"),
            (serde_json::json!({"ticket_id":"FS#12345"}), "FS#12345"),
            (serde_json::json!({"id":"GLIT-42"}), "GLIT-42"),
            (serde_json::json!({"project":"quickask"}), "quickask"),
            (
                serde_json::json!({"command":"playwright-cli click e12"}),
                "playwright-cli click e12",
            ),
            (
                serde_json::json!({"command":"playwright-cli attach --extension=chrome"}),
                "playwright-cli attach --extension=chrome",
            ),
            (
                serde_json::json!({"command":"npx playwright-cli snapshot"}),
                "playwright-cli snapshot",
            ),
            (
                serde_json::json!({"command":"/usr/local/bin/playwright-cli fill e1 hola"}),
                "playwright-cli fill e1 hola",
            ),
            (
                serde_json::json!({"command":"date", "name":"shell"}),
                "date",
            ),
        ] {
            assert_eq!(
                tool_title("shell", &input).as_deref(),
                Some(expected),
                "{input}"
            );
        }
    }
    #[test]
    fn titles_remove_chained_shell_setup_and_collapse_whitespace() {
        for (command, expected) in [
            ("export GITLAB_HOST=git.example.com GLAB_NO_UPDATE_NOTIFIER=1 NO_COLOR=1\nfor iid in 77 78; do glab api …", "for iid in 77 78; do glab api …"),
            ("A=1 B='dos palabras' glab   mr\n list", "glab mr list"),
            ("cd '/tmp/con espacios' && source '/tmp/mi entorno' && export A=1 && B=2 glab mr list", "glab mr list"),
            ("export TOKEN=\"a && b\"; for iid in 1 2; do glab api; done", "for iid in 1 2; do glab api; done"),
            ("source ~/.config/example.env && cd /tmp && playwright-cli   click   e12", "playwright-cli click e12"),
            ("export A=1 \\\n B=2\nplaywright-cli attach --extension=chrome", "playwright-cli attach --extension=chrome"),
            ("export EMPTY= && date", "date"),
        ] {
            assert_eq!(tool_title("shell", &serde_json::json!({"command":command})).as_deref(), Some(expected), "{command}");
        }
        assert_eq!(
            tool_title(
                "skill",
                &serde_json::json!({"name":"  skill\n  con   espacios  "})
            )
            .as_deref(),
            Some("skill con espacios")
        );
        assert_eq!(
            tool_title(
                "subagent",
                &serde_json::json!({"agent":"general", "description":" tarea\n  larga "})
            )
            .as_deref(),
            Some("general: tarea larga")
        );
    }
    #[test]
    fn titles_preserve_real_commands_and_quoted_setup_words() {
        for command in [
            "echo A=b",
            "echo 'cd /tmp && hola'",
            "export PLAIN",
            "cd /tmp || date",
            "source script.sh",
            "1BAD=foo echo hola",
        ] {
            assert_eq!(
                tool_title("shell", &serde_json::json!({"command":command})).as_deref(),
                Some(command)
            );
        }
        assert_eq!(
            tool_title("shell", &serde_json::json!({"command":"export A=1"})),
            None
        );
        assert_eq!(
            tool_title("search", &serde_json::json!({"query":" \n \t "})),
            None
        );
    }
    #[test]
    fn playwright_titles_keep_binary_remove_session_and_stop_at_first_command() {
        for (command, expected) in [
            ("playwright-cli -s=chrome goto https://x", "playwright-cli goto https://x"),
            ("source ~/.config/browser.env && playwright-cli -s=chrome goto https://x 2>&1 | tail -20 && playwright-cli -s=chrome snapshot", "playwright-cli goto https://x 2>&1"),
            ("playwright-cli -s=one snapshot && playwright-cli -s=two click e12", "playwright-cli snapshot"),
            ("playwright-cli list | cat && playwright-cli -s=chrome snapshot", "playwright-cli list"),
            ("echo 'playwright-cli -s=fake close' && playwright-cli -s=chrome snapshot", "playwright-cli snapshot"),
            ("npx playwright-cli -s='mi sesion' fill e1 'a && b | c' | cat", "playwright-cli fill e1 'a && b | c'"),
            ("playwright-cli -s=chrome fill e1 '-s=literal'", "playwright-cli fill e1 '-s=literal'"),
            ("'/usr/local/bin/playwright-cli' -s=chrome snapshot", "playwright-cli snapshot"),
            ("playwright-cli -s=chrome goto 'https://x?a=1&&b=2' && echo fin", "playwright-cli goto 'https://x?a=1&&b=2'"),
        ] {
            assert_eq!(tool_title("shell", &serde_json::json!({"command":command})).as_deref(), Some(expected), "{command}");
        }
        assert_eq!(
            tool_title(
                "shell",
                &serde_json::json!({"command":"echo 'playwright-cli -s=fake close'"})
            )
            .as_deref(),
            Some("echo 'playwright-cli -s=fake close'")
        );
    }
    #[test]
    fn execute_titles_describe_the_code_mode_calls() {
        for (code, expected) in [
            (
                "const b = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 });\nreturn JSON.stringify(b).slice(0, 3000);",
                "freshservice get_ticket ticket_id: 12345",
            ),
            (
                "const c = await tools[\"freshservice-mcp\"].get_ticket_conversations({ ticket_id: 12345 });\nreturn JSON.stringify(c).slice(0, 4000);",
                "freshservice get_ticket_conversations ticket_id: 12345",
            ),
            (
                "const a = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12346 });\nreturn JSON.stringify(a).slice(0, 2500);",
                "freshservice get_ticket ticket_id: 12346",
            ),
            (
                "const t = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 });\nconst d = t?.data ?? t;\nreturn { id: d.id, status: d.status };",
                "freshservice get_ticket ticket_id: 12345",
            ),
            (
                "const [a,b] = await Promise.allSettled([\n  tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12346 }),\n  tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 })\n]);\nreturn { a, b };",
                "freshservice get_ticket ticket_id: 12346 · freshservice get_ticket ticket_id: 12",
            ),
            (
                "const t = await tools['gitlab'].list_issues({ project: 'quickask', state: 'opened' });\nreturn t;",
                "gitlab list_issues project: quickask, state: opened",
            ),
            (
                "return tools.jira.get_issue({ key: \"PROJ-123\" });",
                "jira get_issue key: PROJ-123",
            ),
            (
                "const d = await tools.context7[\"query-docs\"]({ libraryId: '/mongodb/docs', query: 'find' });",
                "context7 query-docs libraryId: /mongodb/docs, query: find",
            ),
            (
                "const b = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 });\nconst c = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 });",
                "freshservice get_ticket ticket_id: 12345",
            ),
            (
                "// solo una llamada\nconst x = await tools.discord.discord_list_guilds({});",
                "discord discord_list_guilds",
            ),
            (
                "const r = await tools.shell ? null : null;",
                "const r = await tools.shell ? null : null;",
            ),
            (
                "const found = search({ query: \"get_ticket freshservice\" });\nreturn found.items;",
                "const found = search({ query: \"get_ticket freshservice\" });",
            ),
            ("return \"sin herramientas\";", "return \"sin herramientas\";"),
        ] {
            assert_eq!(
                tool_title("execute", &serde_json::json!({ "code": code })).as_deref(),
                Some(expected),
                "{code}"
            );
        }
        assert_eq!(
            tool_title("execute", &serde_json::json!({"code": "  \n  "})),
            None
        );
        assert_eq!(tool_title("execute", &serde_json::json!({})), None);
    }
    #[test]
    fn execute_titles_do_not_read_members_inside_strings() {
        for (code, expected) in [
            (
                "const s = \"tools[\\\"freshservice-mcp\\\"].get_ticket({ ticket_id: 1 })\";\nreturn s;",
                "const s = \"tools[\\\"freshservice-mcp\\\"].get_ticket({ ticket_id: 1 })\";",
            ),
            (
                "const paths = ['tools[\"gitlab\"].list_issues'];\nreturn paths;",
                "const paths = ['tools[\"gitlab\"].list_issues'];",
            ),
            (
                "const x = mytools[\"freshservice-mcp\"].get_ticket({ ticket_id: 1 });\nreturn x;",
                "const x = mytools[\"freshservice-mcp\"].get_ticket({ ticket_id: 1 });",
            ),
        ] {
            assert_eq!(
                tool_title("execute", &serde_json::json!({ "code": code })).as_deref(),
                Some(expected),
                "{code}"
            );
        }
    }
    #[test]
    fn execute_success_replaces_a_weak_title_with_the_tool_calls() {
        let mut ctx = TurnCtx {
            session_id: "s".into(),
            turn_id: "turn".into(),
            ..Default::default()
        };
        let call = |id: &str| serde_json::json!({"id":id,"type":"session.tool.input.started","data":{"sessionID":"s","id":"c1","name":"execute"}});
        let UiEvent::Tool {
            status,
            title,
            name,
            ..
        } = &translate(&call("e1"), &mut ctx)[0]
        else {
            panic!("tool esperado");
        };
        assert_eq!((status.as_str(), name.as_str()), ("pending", "execute"));
        assert_eq!(title, &None);
        let called = serde_json::json!({"id":"e2","type":"session.tool.called","data":{"sessionID":"s","id":"c1","input":{"code":"const found = search({ query: \"get_ticket freshservice\" });\nreturn found.items;"}}});
        let UiEvent::Tool {
            status,
            title,
            name,
            ..
        } = &translate(&called, &mut ctx)[0]
        else {
            panic!("tool esperado");
        };
        assert_eq!((status.as_str(), name.as_str()), ("running", "execute"));
        assert_eq!(
            title.as_deref(),
            Some("const found = search({ query: \"get_ticket freshservice\" });")
        );
        let success = serde_json::json!({"id":"e3","type":"session.tool.success","data":{"sessionID":"s","id":"c1","content":[{"type":"text","text":"[]"}],"metadata":{"toolCalls":[{"tool":"search","status":"completed","input":{"query":"get_ticket freshservice"}}],"truncated":false}}});
        let events = translate(&success, &mut ctx);
        assert_eq!(events.len(), 2);
        let UiEvent::Tool {
            status,
            title,
            name,
            ..
        } = &events[0]
        else {
            panic!("tool esperado");
        };
        assert_eq!((status.as_str(), name.as_str()), ("ok", "execute"));
        assert_eq!(
            title.as_deref(),
            Some("const found = search({ query: \"get_ticket freshservice\" });")
        );
        let UiEvent::Tool {
            status,
            title,
            call_id,
            ..
        } = &events[1]
        else {
            panic!("tool esperado");
        };
        assert_eq!(status, "ok");
        assert_eq!(call_id, "c1");
        assert_eq!(
            title.as_deref(),
            Some("search query: get_ticket freshservice")
        );
    }
    #[test]
    fn execute_success_keeps_the_title_when_the_code_already_lists_the_calls() {
        let mut ctx = TurnCtx {
            session_id: "s".into(),
            turn_id: "turn".into(),
            ..Default::default()
        };
        translate(
            &serde_json::json!({"id":"e1","type":"session.tool.input.started","data":{"sessionID":"s","id":"c1","name":"execute"}}),
            &mut ctx,
        );
        translate(
            &serde_json::json!({"id":"e2","type":"session.tool.called","data":{"sessionID":"s","id":"c1","input":{"code":"const t = await tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 });\nreturn JSON.stringify(t);"}}}),
            &mut ctx,
        );
        let success = serde_json::json!({"id":"e3","type":"session.tool.success","data":{"sessionID":"s","id":"c1","metadata":{"toolCalls":[{"tool":"freshservice-mcp.get_ticket","status":"completed","input":{"ticket_id":12345}}]}}});
        let events = translate(&success, &mut ctx);
        assert_eq!(events.len(), 1);
        let UiEvent::Tool { status, title, .. } = &events[0] else {
            panic!("tool esperado");
        };
        assert_eq!(status, "ok");
        assert_eq!(
            title.as_deref(),
            Some("freshservice get_ticket ticket_id: 12345")
        );
    }
    #[test]
    fn tool_events_without_input_keep_the_previous_title() {
        let mut ctx = TurnCtx {
            session_id: "s".into(),
            turn_id: "turn".into(),
            ..Default::default()
        };
        translate(
            &serde_json::json!({"id":"e1","type":"session.tool.input.started","data":{"sessionID":"s","id":"c1","name":"shell"}}),
            &mut ctx,
        );
        translate(
            &serde_json::json!({"id":"e2","type":"session.tool.called","data":{"sessionID":"s","id":"c1","input":{"command":"date"}}}),
            &mut ctx,
        );
        let UiEvent::Tool { status, title, .. } = &translate(
            &serde_json::json!({"id":"e3","type":"session.tool.success","data":{"sessionID":"s","id":"c1","content":[{"type":"text","text":"jue 15 oct"}]}}),
            &mut ctx,
        )[0] else {
            panic!("tool esperado");
        };
        assert_eq!(status, "ok");
        assert_eq!(title.as_deref(), Some("date"));
    }
    #[test]
    fn execute_tool_calls_cover_several_mcp_servers_and_truncate() {
        assert_eq!(
            execute_title("const [a,b] = await Promise.allSettled([\n  tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12346 }),\n  tools[\"freshservice-mcp\"].get_ticket({ ticket_id: 12345 })\n]);").unwrap(),
            "freshservice get_ticket ticket_id: 12346 · freshservice get_ticket ticket_id: 12345"
        );
        let title = tool_calls_title(&serde_json::json!([
            {"tool": "freshservice-mcp.get_ticket", "input": {"ticket_id": 12345}},
            {"tool": "jira.get_issue", "input": {"key": "PROJ-123"}}
        ]))
        .unwrap();
        assert_eq!(
            title,
            "freshservice-mcp.get_ticket ticket_id: 12345 · jira.get_issue key: PROJ-123"
        );
        assert_eq!(tool_calls_title(&serde_json::json!([])), None);
        assert_eq!(tool_calls_title(&serde_json::json!({})), None);
        assert_eq!(tool_calls_title(&serde_json::json!([{"input":{}}])), None);
        assert_eq!(
            tool_title(
                "execute",
                &serde_json::json!({"code": format!("await tools[\"exa\"].web_search_exa({{ query: \"{}\" }});", "x".repeat(120))})
            )
            .unwrap()
            .chars()
            .count(),
            80
        );
    }
    #[test]
    fn fixture_usage_sums_steps_once_even_with_cumulative_updates() {
        let fixture = include_str!("../../fixtures/sse-turno-con-subagente-y-vision.jsonl");
        let events: Vec<Value> = fixture
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let mut ctx = TurnCtx {
            session_id: string(&events[0]["data"], "sessionID"),
            turn_id: "turn".into(),
            ..Default::default()
        };
        let mut last = None;
        for event in events {
            for e in translate(&event, &mut ctx) {
                if matches!(e, UiEvent::Usage { .. }) {
                    last = Some(e);
                }
            }
        }
        let UiEvent::Usage {
            cost,
            tokens_in,
            tokens_out,
            tokens_reasoning,
            tokens_cache,
            ..
        } = last.unwrap()
        else {
            panic!("usage ausente");
        };
        assert!((cost - (0.000287556 + 0.000533106)).abs() < 1e-12);
        assert_eq!(
            (tokens_in, tokens_out, tokens_reasoning, tokens_cache),
            (2681, 370, 124, 40704)
        );
    }
    #[test]
    fn step_usage_updates_without_step_metadata_are_accumulated() {
        let mut ctx = TurnCtx {
            session_id: "s".into(),
            ..Default::default()
        };
        for (id, cost, input, output, reasoning, cache) in
            [("a", 0.1, 10, 3, 2, 4), ("b", 0.2, 20, 5, 6, 8)]
        {
            let e = serde_json::json!({"id":id,"type":"session.usage.updated","data":{"sessionID":"s","cost":cost,"tokens":{"input":input,"output":output,"reasoning":reasoning,"cache":{"read":cache}}}});
            translate(&e, &mut ctx);
            assert!(translate(&e, &mut ctx).is_empty());
        }
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
    #[test]
    fn ended_step_without_started_does_not_duplicate_its_usage_update() {
        let mut ctx = TurnCtx {
            session_id: "s".into(),
            ..Default::default()
        };
        let data = serde_json::json!({"sessionID":"s","assistantMessageID":"m","cost":0.1,"tokens":{"input":10}});
        translate(
            &serde_json::json!({"id":"ended","type":"session.step.ended","data":data}),
            &mut ctx,
        );
        translate(
            &serde_json::json!({"id":"usage","type":"session.usage.updated","data":data}),
            &mut ctx,
        );
        let UiEvent::Usage {
            cost, tokens_in, ..
        } = ctx.usage.event("")
        else {
            panic!();
        };
        assert!((cost - 0.1).abs() < 1e-12);
        assert_eq!(tokens_in, 10);
    }
}
