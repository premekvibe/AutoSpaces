//! Platform-independent core: orchestrator, models and abstractions (no direct Win32 calls).

pub mod carry;
pub mod learned;
pub mod model;
pub mod orchestrator;
pub mod settings;
pub mod state;
pub mod support;

pub use model::{Event, Schedule, State, WindowSystem};
pub use orchestrator::Orchestrator;
pub use settings::Settings;

#[cfg(test)]
mod tests;
