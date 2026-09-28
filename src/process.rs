//! The same process ownership used by the IDE, with a small text-capture helper.
use anyhow::{Context, Result, bail};
pub use cranpose_plugin_process::{Cancellation, ExecutionEvent, execute, execute_observed};
use std::{process::Command, time::Duration};

pub fn capture(command: Command, cancel: &Cancellation) -> Result<String> {
    let output = cranpose_plugin_process::capture(command, None, Duration::from_secs(60), || {
        cancel.is_cancelled()
    })?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr));
    }
    String::from_utf8(output.stdout).context("Command returned non-UTF-8 output")
}
