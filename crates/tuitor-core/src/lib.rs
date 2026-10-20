//! tuitor's debugger engine. It talks DAP to adapters and records every stop,
//! but knows nothing about terminals; the UI renders what it exposes.

pub mod breakpoints;
mod capture;
pub mod config;
pub mod doctor;
mod engine;
pub mod flow;
pub mod install;
mod jsonc;
pub mod log;
pub mod persist;
pub mod process;
pub mod project;
pub mod session;
pub mod snapshot;
pub mod store;
mod tmpdir;
pub mod trace;

pub use engine::{Engine, EngineMsg, EngineOptions, Level, Notice, Step};
