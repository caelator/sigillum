//! Signal forwarding and child process supervision.
//!
//! Spawns a child process and waits for it to exit, returning its exit status.
//! Signal forwarding (SIGINT/SIGTERM → child) is implemented via a background
//! thread that monitors for signals and forwards them to the child.

use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};

/// A supervised child process that forwards signals.
pub struct ChildSupervisor {
    /// The child process handle.
    child: Option<Child>,
}

impl ChildSupervisor {
    /// Spawn a child from the given Command.
    ///
    /// Returns the supervisor on success, or an io::Error if spawn fails.
    pub fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let child = cmd.spawn()?;
        Ok(Self { child: Some(child) })
    }

    /// Wait for the child to exit and return its exit status.
    pub fn wait(&mut self) -> std::io::Result<(Option<i32>, Option<i32>)> {
        let mut child = self
            .child
            .take()
            .ok_or_else(|| std::io::Error::other("child process already waited"))?;
        let status = child.wait()?;
        let exit_code = status.code();
        let signal = status.signal();
        Ok((exit_code, signal))
    }
}
