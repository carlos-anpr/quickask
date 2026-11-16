//! Salida de seguridad del turno: corta al agente si se atasca, repite el mismo paso
//! o se eterniza, en vez de dejarlo reintentando indefinidamente.
use crate::events::UiEvent;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub turn: Duration,
    pub stall: Duration,
    pub max_tools: usize,
    pub max_repeats: usize,
}
impl Limits {
    pub fn from_config(c: &crate::config::Config) -> Self {
        Self {
            turn: Duration::from_secs(c.turn_timeout_secs),
            stall: Duration::from_secs(c.stall_timeout_secs),
            max_tools: c.max_tool_calls,
            max_repeats: c.max_step_repeats,
        }
    }
    /// Límites del resumen tras un corte: breve y sin herramientas.
    pub fn summary() -> Self {
        Self {
            turn: Duration::from_secs(60),
            stall: Duration::from_secs(45),
            max_tools: 2,
            max_repeats: 2,
        }
    }
}

pub struct Watchdog {
    limits: Limits,
    started: Instant,
    last_event: Instant,
    tools: HashSet<String>,
    steps: HashMap<String, HashSet<String>>,
}
impl Watchdog {
    pub fn new(limits: Limits, now: Instant) -> Self {
        Self {
            limits,
            started: now,
            last_event: now,
            tools: HashSet::new(),
            steps: HashMap::new(),
        }
    }
    /// Registra un evento del turno; devuelve el motivo de corte si toca parar.
    pub fn observe(&mut self, ev: &UiEvent, now: Instant) -> Option<String> {
        self.last_event = now;
        let UiEvent::Tool {
            call_id,
            name,
            title,
            ..
        } = ev
        else {
            return None;
        };
        if self.tools.insert(call_id.clone()) && self.tools.len() > self.limits.max_tools {
            return Some(format!(
                "demasiadas acciones (más de {})",
                self.limits.max_tools
            ));
        }
        // El mismo paso = misma herramienta y mismo arranque de título, en llamadas distintas.
        let Some(title) = title else { return None };
        let key = format!("{name} {}", step_key(title));
        // Un mismo callId emite varios eventos (pending/running/ok): se cuenta una vez por llamada.
        let calls = self.steps.entry(key).or_default();
        calls.insert(call_id.clone());
        let calls = calls.len();
        (calls >= self.limits.max_repeats).then(|| {
            format!(
                "repite el mismo paso {} veces ({})",
                calls,
                title.chars().take(60).collect::<String>()
            )
        })
    }
    /// Comprobación periódica de tiempo total y de falta de avance.
    pub fn tick(&self, now: Instant) -> Option<String> {
        if now.duration_since(self.started) > self.limits.turn {
            return Some(format!(
                "superó el tiempo máximo ({})",
                human(self.limits.turn)
            ));
        }
        if now.duration_since(self.last_event) > self.limits.stall {
            return Some(format!("lleva {} sin avanzar", human(self.limits.stall)));
        }
        None
    }
}
/// Las tres primeras palabras del título: `playwright-cli attach --extension=chrome …`
/// cuenta como el mismo paso aunque cambie la cola del comando.
fn step_key(title: &str) -> String {
    let first = title.split([';', '&', '|']).next().unwrap_or(title);
    first
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}
fn human(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 && s.is_multiple_of(60) {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    }
}
pub fn summary_prompt(reason: &str) -> String {
    format!(
        "[quickask] Te he detenido por seguridad: {reason}. NO uses herramientas. \
         En 2-4 líneas: qué has conseguido hasta ahora (con los datos concretos si los tienes), \
         qué ha fallado y qué puede hacer el usuario a continuación."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool(id: &str, title: &str, status: &str) -> UiEvent {
        UiEvent::Tool {
            turn_id: "t".into(),
            call_id: id.into(),
            name: "shell".into(),
            status: status.into(),
            title: Some(title.into()),
        }
    }
    fn limits() -> Limits {
        Limits {
            turn: Duration::from_secs(300),
            stall: Duration::from_secs(180),
            max_tools: 40,
            max_repeats: 3,
        }
    }
    #[test]
    fn same_step_three_times_trips_but_status_updates_do_not_count() {
        let now = Instant::now();
        let mut w = Watchdog::new(limits(), now);
        for status in ["pending", "running", "ok"] {
            assert!(w
                .observe(
                    &tool("a", "playwright-cli attach --extension=chrome", status),
                    now
                )
                .is_none());
        }
        assert!(w
            .observe(
                &tool(
                    "b",
                    "playwright-cli attach --extension=chrome >/dev/null",
                    "running"
                ),
                now
            )
            .is_none());
        let r = w
            .observe(
                &tool(
                    "c",
                    "playwright-cli attach --extension=chrome; sleep 1",
                    "running",
                ),
                now,
            )
            .unwrap();
        assert!(r.contains("repite el mismo paso 3 veces"), "{r}");
    }
    #[test]
    fn different_steps_do_not_trip() {
        let now = Instant::now();
        let mut w = Watchdog::new(limits(), now);
        for (i, t) in [
            "playwright-cli goto https://a",
            "playwright-cli snapshot",
            "playwright-cli fill e12 logs",
            "playwright-cli press Enter",
        ]
        .iter()
        .enumerate()
        {
            assert!(w
                .observe(&tool(&i.to_string(), t, "running"), now)
                .is_none());
        }
    }
    #[test]
    fn too_many_tools_trip() {
        let now = Instant::now();
        let mut w = Watchdog::new(
            Limits {
                max_tools: 3,
                ..limits()
            },
            now,
        );
        for i in 0..3 {
            assert!(w
                .observe(&tool(&i.to_string(), &format!("cmd{i}"), "running"), now)
                .is_none());
        }
        assert!(w
            .observe(&tool("x", "otro", "running"), now)
            .unwrap()
            .contains("demasiadas acciones"));
    }
    #[test]
    fn time_and_stall() {
        let now = Instant::now();
        let mut w = Watchdog::new(limits(), now);
        assert!(w.tick(now + Duration::from_secs(100)).is_none());
        assert!(w
            .tick(now + Duration::from_secs(181))
            .unwrap()
            .contains("3 min sin avanzar"));
        w.observe(
            &UiEvent::Variant {
                variant: String::new(),
            },
            now + Duration::from_secs(250),
        );
        assert!(w.tick(now + Duration::from_secs(260)).is_none());
        assert!(w
            .tick(now + Duration::from_secs(301))
            .unwrap()
            .contains("tiempo máximo (5 min)"));
    }
}
