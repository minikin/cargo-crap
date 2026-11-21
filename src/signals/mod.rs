//! Structural maintainability signals and their Rust syntax walker.

pub mod compute;
pub mod config;
pub mod model;

pub use compute::compute;
pub use model::{Limits, Measured, Signal, SignalSettings, Signals, Spans};
