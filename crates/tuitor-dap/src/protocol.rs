//! Message envelopes shared by requests, responses and events.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProtocolMessage {
    Request(RequestMessage),
    Response(ResponseMessage),
    Event(EventMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMessage {
    pub seq: i64,
    pub command: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub arguments: Value,
}

/// Note: the spec spells `request_seq` in snake_case, unlike every other field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub seq: i64,
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage {
    pub seq: i64,
    pub event: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_message_kind() {
        let req: ProtocolMessage =
            serde_json::from_str(r#"{"seq":1,"type":"request","command":"runInTerminal","arguments":{}}"#)
                .expect("request");
        assert!(matches!(req, ProtocolMessage::Request(r) if r.command == "runInTerminal"));

        let resp: ProtocolMessage = serde_json::from_str(
            r#"{"seq":2,"type":"response","request_seq":1,"success":false,"command":"launch","message":"nope"}"#,
        )
        .expect("response");
        assert!(matches!(resp, ProtocolMessage::Response(r) if !r.success && r.body.is_null()));

        let ev: ProtocolMessage =
            serde_json::from_str(r#"{"seq":3,"type":"event","event":"initialized"}"#).expect("event");
        assert!(matches!(ev, ProtocolMessage::Event(e) if e.event == "initialized"));
    }

    #[test]
    fn omits_empty_arguments() {
        let msg =
            ProtocolMessage::Request(RequestMessage { seq: 7, command: "threads".into(), arguments: Value::Null });
        assert_eq!(
            serde_json::to_string(&msg).expect("serialize"),
            r#"{"type":"request","seq":7,"command":"threads"}"#
        );
    }
}
