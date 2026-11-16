use serde::Serialize;
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UiEvent {
    Summoned {
        greeting: String,
    },
    Hidden,
    Backend {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    TurnStarted {
        turn_id: String,
        text: String,
        screen: bool,
    },
    Screen {
        turn_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thumb: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Reasoning {
        turn_id: String,
        message_id: String,
        delta: String,
    },
    Text {
        turn_id: String,
        message_id: String,
        ordinal: u64,
        delta: String,
    },
    TextFinal {
        turn_id: String,
        message_id: String,
        ordinal: u64,
        text: String,
    },
    Tool {
        turn_id: String,
        call_id: String,
        name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Permission {
        turn_id: String,
        request_id: String,
        action: String,
        resources: Vec<String>,
    },
    PermissionClosed {
        request_id: String,
    },
    Usage {
        turn_id: String,
        cost: f64,
        tokens_in: u64,
        tokens_out: u64,
        tokens_reasoning: u64,
        tokens_cache: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        tokens_context: Option<u64>,
    },
    Variant {
        variant: String,
    },
    Model {
        model: String,
        fallback_model: String,
    },
    Notice {
        turn_id: String,
        text: String,
    },
    Done {
        turn_id: String,
        outcome: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    ConversationReset,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r8_events_match_camel_case_contract() {
        assert_eq!(
            serde_json::to_value(UiEvent::Variant {
                variant: "high".into()
            })
            .unwrap(),
            serde_json::json!({"kind":"variant","variant":"high"})
        );
        assert_eq!(
            serde_json::to_value(UiEvent::Model {
                model: "opencode/deepseek-v4.1-flash-free".into(),
                fallback_model: "opencode-go/deepseek-v4.1-flash".into()
            })
            .unwrap(),
            serde_json::json!({"kind":"model","model":"opencode/deepseek-v4.1-flash-free","fallbackModel":"opencode-go/deepseek-v4.1-flash"})
        );
        assert_eq!(
            serde_json::to_value(UiEvent::Notice {
                turn_id: "t".into(),
                text: "Flash Free no disponible; reintentando con Flash Go".into()
            })
            .unwrap(),
            serde_json::json!({"kind":"notice","turnId":"t","text":"Flash Free no disponible; reintentando con Flash Go"})
        );
        assert_eq!(
            serde_json::to_value(UiEvent::Usage {
                turn_id: "t".into(),
                cost: 0.1,
                tokens_in: 2,
                tokens_out: 3,
                tokens_reasoning: 4,
                tokens_cache: 5,
                tokens_context: None
            })
            .unwrap(),
            serde_json::json!({"kind":"usage","turnId":"t","cost":0.1,"tokensIn":2,"tokensOut":3,"tokensReasoning":4,"tokensCache":5})
        );
    }
}
