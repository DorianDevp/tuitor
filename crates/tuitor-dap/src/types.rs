//! Typed arguments and bodies for the subset of DAP that tuitor speaks.
//!
//! Field names follow the specification; everything an adapter may omit is
//! optional or defaulted so that partial adapters still deserialize.

use serde::{Deserialize, Serialize};

/// Arguments or bodies without fields. Accepts any object (extra fields are ignored).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Empty {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeArguments {
    #[serde(rename = "clientID")]
    pub client_id: String,
    pub client_name: String,
    #[serde(rename = "adapterID")]
    pub adapter_id: String,
    pub locale: String,
    pub lines_start_at1: bool,
    pub columns_start_at1: bool,
    pub path_format: String,
    pub supports_variable_type: bool,
    pub supports_variable_paging: bool,
    pub supports_run_in_terminal_request: bool,
    pub supports_start_debugging_request: bool,
}

impl InitializeArguments {
    pub fn new(adapter_id: impl Into<String>) -> Self {
        Self {
            client_id: "tuitor".into(),
            client_name: "tuitor".into(),
            adapter_id: adapter_id.into(),
            locale: "en-US".into(),
            lines_start_at1: true,
            columns_start_at1: true,
            path_format: "path".into(),
            supports_variable_type: true,
            supports_variable_paging: false,
            supports_run_in_terminal_request: false,
            supports_start_debugging_request: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub supports_configuration_done_request: bool,
    pub supports_conditional_breakpoints: bool,
    pub supports_hit_conditional_breakpoints: bool,
    pub supports_log_points: bool,
    pub supports_terminate_request: bool,
    pub supports_restart_request: bool,
    pub supports_evaluate_for_hovers: bool,
    pub supports_set_variable: bool,
    pub exception_breakpoint_filters: Vec<ExceptionBreakpointsFilter>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionBreakpointsFilter {
    pub filter: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint {
    pub line: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsArguments {
    pub source: Source,
    pub breakpoints: Vec<SourceBreakpoint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SetBreakpointsResponse {
    pub breakpoints: Vec<Breakpoint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub message: Option<String>,
    pub source: Option<Source>,
    pub line: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetExceptionBreakpointsArguments {
    pub filters: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ThreadsResponse {
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thread {
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceArguments {
    pub thread_id: i64,
    pub start_frame: i64,
    pub levels: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackTraceResponse {
    pub stack_frames: Vec<StackFrame>,
    pub total_frames: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub line: i64,
    #[serde(default)]
    pub column: i64,
    /// `normal`, `label` or `subtle` (library / vendor code).
    #[serde(default)]
    pub presentation_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopesArguments {
    pub frame_id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScopesResponse {
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    #[serde(default)]
    pub presentation_hint: Option<String>,
    pub variables_reference: i64,
    #[serde(default)]
    pub expensive: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesArguments {
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct VariablesResponse {
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub evaluate_name: Option<String>,
    #[serde(default)]
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateArguments {
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<i64>,
    /// `watch`, `repl`, `hover` or `clipboard`.
    pub context: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateResponse {
    pub result: String,
    #[serde(default, rename = "type")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub variables_reference: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArguments {
    pub thread_id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinueResponse {
    pub all_threads_continued: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectArguments {
    pub terminate_debuggee: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceArguments {
    pub source: Source,
    pub source_reference: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResponse {
    pub content: String,
}

// ---- event bodies ---------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoppedEventBody {
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<i64>,
    pub text: Option<String>,
    pub all_threads_stopped: Option<bool>,
    pub hit_breakpoint_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinuedEventBody {
    pub thread_id: Option<i64>,
    pub all_threads_continued: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExitedEventBody {
    pub exit_code: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadEventBody {
    pub reason: String,
    pub thread_id: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputEventBody {
    /// `console`, `important`, `stdout`, `stderr` or `telemetry`.
    pub category: Option<String>,
    pub output: String,
    pub source: Option<Source>,
    pub line: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BreakpointEventBody {
    pub reason: String,
    pub breakpoint: Breakpoint,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessEventBody {
    pub name: String,
    pub system_process_id: Option<i64>,
}
