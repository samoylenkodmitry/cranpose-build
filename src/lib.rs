//! Local cross-platform builds with explicit toolchains and owned subprocesses.
pub mod config;
pub mod download;
pub mod engine;
pub mod platform;
pub mod process;
pub mod project;
pub mod tools;

pub use config::{Config, Request};
pub use engine::{Artifact, Event, build, plan, run};
pub use platform::{Host, Platform};
pub use process::Cancellation;
