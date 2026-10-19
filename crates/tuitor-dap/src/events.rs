//! Typed view of the events tuitor reacts to. Everything else stays raw.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::protocol::EventMessage;
use crate::types::*;

#[derive(Debug, Clone)]
pub enum Event {
    Initialized,
    Stopped(StoppedEventBody),
    Continued(ContinuedEventBody),
    Exited(ExitedEventBody),
    Terminated,
    Thread(ThreadEventBody),
    Output(OutputEventBody),
    Breakpoint(BreakpointEventBody),
    Process(ProcessEventBody),
    Other { event: String, body: Value },
}

impl Event {
    pub fn name(&self) -> &str {
        match self {
            Event::Initialized => "initialized",
            Event::Stopped(_) => "stopped",
            Event::Continued(_) => "continued",
            Event::Exited(_) => "exited",
            Event::Terminated => "terminated",
            Event::Thread(_) => "thread",
            Event::Output(_) => "output",
            Event::Breakpoint(_) => "breakpoint",
            Event::Process(_) => "process",
            Event::Other { event, .. } => event,
        }
    }
}

impl TryFrom<EventMessage> for Event {
    type Error = serde_json::Error;

    fn try_from(msg: EventMessage) -> Result<Self, Self::Error> {
        fn body<T: DeserializeOwned>(v: Value) -> Result<T, serde_json::Error> {
            serde_json::from_value(if v.is_null() { Value::Object(Default::default()) } else { v })
        }
        Ok(match msg.event.as_str() {
            "initialized" => Event::Initialized,
            "stopped" => Event::Stopped(body(msg.body)?),
            "continued" => Event::Continued(body(msg.body)?),
            "exited" => Event::Exited(body(msg.body)?),
            "terminated" => Event::Terminated,
            "thread" => Event::Thread(body(msg.body)?),
            "output" => Event::Output(body(msg.body)?),
            "breakpoint" => Event::Breakpoint(body(msg.body)?),
            "process" => Event::Process(body(msg.body)?),
            _ => Event::Other { event: msg.event, body: msg.body },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Event {
        let msg: EventMessage = serde_json::from_str(json).expect("event message");
        Event::try_from(msg).expect("typed event")
    }

    #[test]
    fn stopped_with_hit_breakpoints() {
        let ev =
            parse(r#"{"seq":5,"event":"stopped","body":{"reason":"breakpoint","threadId":1,"hitBreakpointIds":[3]}}"#);
        let Event::Stopped(body) = ev else { panic!("expected stopped") };
        assert_eq!(body.reason, "breakpoint");
        assert_eq!(body.thread_id, Some(1));
        assert_eq!(body.hit_breakpoint_ids, [3]);
    }

    #[test]
    fn events_without_body() {
        assert!(matches!(parse(r#"{"seq":1,"event":"terminated"}"#), Event::Terminated));
        assert!(matches!(parse(r#"{"seq":1,"event":"continued"}"#), Event::Continued(_)));
    }

    #[test]
    fn unknown_events_are_kept() {
        let ev = parse(r#"{"seq":1,"event":"loadedSource","body":{"reason":"new"}}"#);
        assert_eq!(ev.name(), "loadedSource");
    }
}
