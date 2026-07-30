//! Child process supervision for `sigillum run`.
//!
//! Spawns a child process, explicitly forwards SIGINT and SIGTERM to its PID,
//! and waits for it to exit before returning its terminal status.

use std::io::{self, ErrorKind};
use std::mem;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static SUPERVISOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static CHILD_PID: AtomicI32 = AtomicI32::new(0);
static PENDING_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(signal: libc::c_int) {
    PENDING_SIGNAL.store(signal, Ordering::SeqCst);

    let child_pid = CHILD_PID.load(Ordering::SeqCst);
    if child_pid > 0 {
        // SAFETY: `kill` is async-signal-safe, and `child_pid` is the positive PID
        // published by the active supervisor.
        unsafe {
            libc::kill(child_pid, signal);
        }
    }
}

fn managed_signal_set() -> io::Result<libc::sigset_t> {
    // SAFETY: An all-zero `sigset_t` is immediately initialized by
    // `sigemptyset` before it is read.
    let mut signals = unsafe { mem::zeroed::<libc::sigset_t>() };

    // SAFETY: `signals` points to valid writable storage.
    if unsafe { libc::sigemptyset(&mut signals) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `signals` is an initialized signal set and SIGINT is valid.
    if unsafe { libc::sigaddset(&mut signals, libc::SIGINT) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `signals` is an initialized signal set and SIGTERM is valid.
    if unsafe { libc::sigaddset(&mut signals, libc::SIGTERM) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(signals)
}

fn block_managed_signals() -> io::Result<libc::sigset_t> {
    let signals = managed_signal_set()?;
    // SAFETY: An all-zero `sigset_t` is valid writable storage for the previous
    // mask returned by `pthread_sigmask`.
    let mut previous_mask = unsafe { mem::zeroed::<libc::sigset_t>() };

    // SAFETY: Both signal-set pointers are valid for the duration of the call.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &signals, &mut previous_mask) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }

    Ok(previous_mask)
}

fn restore_signal_mask(previous_mask: &libc::sigset_t) -> io::Result<()> {
    // SAFETY: `previous_mask` was initialized by `pthread_sigmask` on this
    // thread, and the old-mask output is intentionally unused.
    let result =
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, previous_mask, ptr::null_mut()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }

    Ok(())
}

fn install_signal_action(signal: libc::c_int) -> io::Result<libc::sigaction> {
    // SAFETY: Both values are initialized below before any field is read by
    // `sigaction`.
    let (mut action, mut previous_action) = unsafe {
        (
            mem::zeroed::<libc::sigaction>(),
            mem::zeroed::<libc::sigaction>(),
        )
    };

    action.sa_sigaction = forward_signal as usize;
    action.sa_flags = 0;
    // SAFETY: `action.sa_mask` is valid writable storage.
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `action` and `previous_action` are valid for the duration of the
    // call, and `signal` is SIGINT or SIGTERM.
    if unsafe { libc::sigaction(signal, &action, &mut previous_action) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(previous_action)
}

fn restore_signal_action(signal: libc::c_int, previous_action: &libc::sigaction) -> io::Result<()> {
    // SAFETY: `previous_action` was populated by a successful `sigaction` call,
    // and the old-action output is intentionally unused.
    if unsafe { libc::sigaction(signal, previous_action, ptr::null_mut()) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

struct SignalState {
    previous_sigint: Option<libc::sigaction>,
    previous_sigterm: Option<libc::sigaction>,
    previous_mask: libc::sigset_t,
    mask_blocked: bool,
    owns_supervisor_slot: bool,
}

impl SignalState {
    fn install() -> io::Result<Self> {
        let previous_mask = block_managed_signals()?;
        if SUPERVISOR_ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            restore_signal_mask(&previous_mask)?;
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "another child supervisor is already active",
            ));
        }

        CHILD_PID.store(0, Ordering::SeqCst);
        PENDING_SIGNAL.store(0, Ordering::SeqCst);

        let mut state = Self {
            previous_sigint: None,
            previous_sigterm: None,
            previous_mask,
            mask_blocked: true,
            owns_supervisor_slot: true,
        };
        state.previous_sigint = Some(install_signal_action(libc::SIGINT)?);
        state.previous_sigterm = Some(install_signal_action(libc::SIGTERM)?);
        Ok(state)
    }

    fn child_signal_mask(&self) -> libc::sigset_t {
        self.previous_mask
    }

    fn publish_child(&self, child_pid: libc::pid_t) {
        CHILD_PID.store(child_pid, Ordering::SeqCst);

        let pending_signal = PENDING_SIGNAL.swap(0, Ordering::SeqCst);
        if pending_signal != 0 {
            // SAFETY: `child_pid` is the positive PID returned by the successful
            // spawn, and `pending_signal` came from a SIGINT/SIGTERM handler.
            unsafe {
                libc::kill(child_pid, pending_signal);
            }
        }
    }

    fn unblock_after_spawn(&mut self) -> io::Result<()> {
        restore_signal_mask(&self.previous_mask)?;
        self.mask_blocked = false;
        Ok(())
    }

    fn restore_actions(&mut self) -> io::Result<()> {
        let mut first_error = None;

        if let Some(previous_action) = self.previous_sigterm.as_ref() {
            match restore_signal_action(libc::SIGTERM, previous_action) {
                Ok(()) => self.previous_sigterm = None,
                Err(error) => first_error = Some(error),
            }
        }
        if let Some(previous_action) = self.previous_sigint.as_ref() {
            match restore_signal_action(libc::SIGINT, previous_action) {
                Ok(()) => self.previous_sigint = None,
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }

        if self.previous_sigint.is_none()
            && self.previous_sigterm.is_none()
            && self.owns_supervisor_slot
        {
            SUPERVISOR_ACTIVE.store(false, Ordering::SeqCst);
            self.owns_supervisor_slot = false;
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if !self.owns_supervisor_slot
            && self.previous_sigint.is_none()
            && self.previous_sigterm.is_none()
            && !self.mask_blocked
        {
            return Ok(());
        }

        let was_blocked = self.mask_blocked;
        let mut first_error = None;

        if !was_blocked {
            match block_managed_signals() {
                Ok(previous_mask) => {
                    self.previous_mask = previous_mask;
                    self.mask_blocked = true;
                }
                Err(error) => first_error = Some(error),
            }
        }

        CHILD_PID.store(0, Ordering::SeqCst);
        PENDING_SIGNAL.store(0, Ordering::SeqCst);

        if was_blocked {
            if let Err(error) = self.restore_actions() {
                first_error.get_or_insert(error);
            }
            if self.mask_blocked {
                match restore_signal_mask(&self.previous_mask) {
                    Ok(()) => self.mask_blocked = false,
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
        } else {
            if self.mask_blocked {
                match restore_signal_mask(&self.previous_mask) {
                    Ok(()) => self.mask_blocked = false,
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if let Err(error) = self.restore_actions() {
                first_error.get_or_insert(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn restore(mut self) -> io::Result<()> {
        self.cleanup()
    }
}

impl Drop for SignalState {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// A supervised child process that forwards signals.
pub struct ChildSupervisor {
    /// The child process handle.
    child: Option<Child>,
    signal_state: Option<SignalState>,
}

impl ChildSupervisor {
    /// Spawn a child from the given Command.
    ///
    /// Installs SIGINT/SIGTERM forwarding before spawning to ensure a signal
    /// cannot terminate the parent and orphan the child. Returns the supervisor
    /// on success, or an io::Error if setup or spawn fails.
    pub fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let mut signal_state = SignalState::install()?;
        let child_signal_mask = signal_state.child_signal_mask();
        // SAFETY: The closure runs after fork and calls only the async-signal-safe
        // `sigprocmask`, restoring the mask captured before SIGINT/SIGTERM were
        // blocked. It neither allocates nor locks.
        unsafe {
            cmd.pre_exec(move || {
                // SAFETY: `child_signal_mask` is an initialized signal set
                // captured by value, and the old-mask output is unused.
                let result =
                    libc::sigprocmask(libc::SIG_SETMASK, &child_signal_mask, ptr::null_mut());
                if result == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }

        let mut child = cmd.spawn()?;
        let child_pid = child.id() as libc::pid_t;
        signal_state.publish_child(child_pid);
        if let Err(error) = signal_state.unblock_after_spawn() {
            CHILD_PID.store(0, Ordering::SeqCst);
            // SAFETY: `child_pid` belongs to the child just spawned by this
            // supervisor. SIGKILL ensures an unreportable setup failure cannot
            // leave it running.
            unsafe {
                libc::kill(child_pid, libc::SIGKILL);
            }
            loop {
                match child.wait() {
                    Err(wait_error) if wait_error.kind() == ErrorKind::Interrupted => continue,
                    _ => break,
                }
            }
            return Err(error);
        }

        Ok(Self {
            child: Some(child),
            signal_state: Some(signal_state),
        })
    }

    /// Wait for and reap the child, forwarding SIGINT/SIGTERM while it runs,
    /// then return its exit code and terminating signal.
    pub fn wait(&mut self) -> std::io::Result<(Option<i32>, Option<i32>)> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("child process already waited"))?;

        let wait_result = loop {
            match child.wait() {
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                result => break result,
            }
        };
        if wait_result.is_ok() {
            self.child = None;
        }

        let restore_result = self
            .signal_state
            .take()
            .ok_or_else(|| io::Error::other("signal handlers already restored"))?
            .restore();

        let status = match (wait_result, restore_result) {
            (Ok(status), Ok(())) => status,
            (Err(error), _) | (Ok(_), Err(error)) => return Err(error),
        };
        let exit_code = status.code();
        let signal = status.signal();
        Ok((exit_code, signal))
    }
}
