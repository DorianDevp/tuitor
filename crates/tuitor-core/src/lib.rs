//! tuitor's debugger engine. It talks DAP to adapters and records every stop,
//! but knows nothing about terminals; the UI renders what it exposes.

pub mod breakpoints;
mod capture;
pub mod config;
mod engine;
mod jsonc;
pub mod log;
pub mod persist;
pub mod session;
pub mod snapshot;

pub use engine::{Engine, EngineMsg, EngineOptions, Level, Notice, Step};
