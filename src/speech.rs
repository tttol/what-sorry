use anyhow::{Context, Result, bail};
use std::process::Command;

pub(crate) struct Speaker;
impl Speaker {
    pub(crate) fn new() -> Self {
        Self
    }
    pub(crate) fn speak(&self, text: &str) -> Result<()> {
        let status = Command::new("/usr/bin/say")
            .args(["-v", "Samantha", "-r", "175", text])
            .status()
            .context("Could not start macOS speech")?;
        if status.success() {
            Ok(())
        } else {
            bail!("macOS speech failed")
        }
    }
}
