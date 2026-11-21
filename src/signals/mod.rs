pub mod compute;

pub use cargo_crap_core::signals::{config, model};
pub use compute::compute;
pub use model::{Limits, Measured, Signal, SignalSettings, Signals, Spans};
