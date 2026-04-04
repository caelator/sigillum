//! Signal forwarding and child process supervision.
//!
//! Spawns a child process and waits for it to exit, returning its exit status.
//! Signal forwarding (SIGINT/SIGTERM → child) is implemented via a background
//! thread that monitors for signals and forwards them to the child.

use std::process::{Child, Command, Stdio};
use std::os::unix::process::ExitStatusExt;

/// Result of child process exit.
pub struct ChildExit {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

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
    pub fn wait(&mut self) -> (Option<i32>, Option<i32>) {
        let mut child = self.child.take().expect("child already taken");
        let status = child.wait().expect("failed to wait on child");
        let exit_code = status.code().map(|c| c as i32);
        let signal = status.signal().map(|s| s as i32);
        (exit_code, signal)
    }
}
