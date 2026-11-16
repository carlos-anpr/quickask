use crate::events::UiEvent;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Default)]
struct Usage {
    cost: f64,
    input: u64,
    output: u64,
    reasoning: u64,
    cache: u64,
}
impl Usage {
    fn from_value(value: &Value) -> Option<Self> {
        if !value["tokens"].is_object() && !value["cost"].is_number() {
            return None;
        }
        Some(Self {
            cost: value["cost"].as_f64().unwrap_or_default(),
            input: value["tokens"]["input"].as_u64().unwrap_or_default(),
            output: value["tokens"]["output"].as_u64().unwrap_or_default(),
            reasoning: value["tokens"]["reasoning"].as_u64().unwrap_or_default(),
            cache: value["tokens"]["cache"]["read"]
                .as_u64()
                .unwrap_or_default(),
        })
    }
    fn add(&mut self, other: &Self) {
        self.cost += other.cost;
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.cache = self.cache.saturating_add(other.cache);
    }
}
#[derive(Default)]
pub struct TurnUsage {
    steps: HashMap<String, Usage>,
    authoritative: HashSet<String>,
    unattributed: Vec<Usage>,
    context: Option<u64>,
}
impl TurnUsage {
    pub fn step(&mut self, id: &str, value: &Value) -> bool {
        if let Some(usage) = Usage::from_value(value) {
            // Context is the latest request footprint, not accumulated billing.
            self.context = Some(
                usage
                    .input
                    .saturating_add(usage.output)
                    .saturating_add(usage.reasoning)
                    .saturating_add(usage.cache)
                    .saturating_add(
                        value["tokens"]["cache"]["write"]
                            .as_u64()
                            .unwrap_or_default(),
                    ),
            );
            self.steps.insert(id.into(), usage);
            self.authoritative.insert(id.into());
            true
        } else {
            false
        }
    }
    pub fn update(&mut self, current_step: Option<&str>, value: &Value) {
        if let Some(usage) = Usage::from_value(value) {
            if let Some(id) = current_step {
                // step.ended/history are authoritative. Older OpenCode versions send
                // a session cumulative usage.updated after each completed step.
                if !self.authoritative.contains(id) {
                    self.steps.insert(id.into(), usage);
                }
            } else {
                self.unattributed.push(usage);
            }
        }
    }
    pub fn reconciled(&mut self) {
        self.unattributed.clear();
    }
    pub fn event(&self, turn: &str) -> UiEvent {
        let mut total = Usage::default();
        for usage in self.steps.values().chain(self.unattributed.iter()) {
            total.add(usage);
        }
        UiEvent::Usage {
            turn_id: turn.into(),
            cost: total.cost,
            tokens_in: total.input,
            tokens_out: total.output,
            tokens_reasoning: total.reasoning,
            tokens_cache: total.cache,
            tokens_context: self.context,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_uses_latest_step_including_cache_without_accumulating() {
        let mut usage = TurnUsage::default();
        usage.step("first", &json!({"tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":200,"write":10}}}));
        usage.step("second", &json!({"tokens":{"input":50,"output":10,"reasoning":2,"cache":{"read":300,"write":8}}}));
        usage.update(Some("second"), &json!({"tokens":{"input":99999}}));
        let event = serde_json::to_value(usage.event("t")).unwrap();
        assert_eq!(event["tokensContext"], 370);
        assert_eq!(event["tokensIn"], 150);
    }
}
