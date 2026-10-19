//! Requests as types: `client.request::<StackTrace>(args)` checks the argument
//! and response types at compile time.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::types::*;

pub trait Request {
    const COMMAND: &'static str;
    type Arguments: Serialize + Send;
    type Response: DeserializeOwned + Send + 'static;
}

macro_rules! requests {
    ($($name:ident => $command:literal, $args:ty, $resp:ty;)*) => {
        $(
            pub enum $name {}
            impl Request for $name {
                const COMMAND: &'static str = $command;
                type Arguments = $args;
                type Response = $resp;
            }
        )*
    };
}

requests! {
    Initialize => "initialize", InitializeArguments, Capabilities;
    // launch/attach arguments are adapter specific, taken verbatim from the launch config.
    Launch => "launch", Value, Empty;
    Attach => "attach", Value, Empty;
    ConfigurationDone => "configurationDone", Empty, Empty;
    SetBreakpoints => "setBreakpoints", SetBreakpointsArguments, SetBreakpointsResponse;
    SetExceptionBreakpoints => "setExceptionBreakpoints", SetExceptionBreakpointsArguments, Empty;
    Threads => "threads", Empty, ThreadsResponse;
    StackTrace => "stackTrace", StackTraceArguments, StackTraceResponse;
    Scopes => "scopes", ScopesArguments, ScopesResponse;
    Variables => "variables", VariablesArguments, VariablesResponse;
    Evaluate => "evaluate", EvaluateArguments, EvaluateResponse;
    Continue => "continue", ThreadArguments, ContinueResponse;
    Next => "next", ThreadArguments, Empty;
    StepIn => "stepIn", ThreadArguments, Empty;
    StepOut => "stepOut", ThreadArguments, Empty;
    Pause => "pause", ThreadArguments, Empty;
    Terminate => "terminate", Empty, Empty;
    Disconnect => "disconnect", DisconnectArguments, Empty;
    SourceContent => "source", SourceArguments, SourceResponse;
}
