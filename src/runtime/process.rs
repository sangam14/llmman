//! Centralized process lifecycle helpers.
//!
//! Every place in llmman that needs to check whether a PID is alive or
//! send it a signal was previously doing its own inline `unsafe
//! libc::kill` block. This module provides safe wrappers with correct
//! EPERM handling and graceful-termination semantics.

use anyhow::Result;
use std::time::Duration;

/// Returns `true` if the given PID corresponds to a live process.
///
/// On Unix, `kill(pid, 0)` probes without actually delivering a signal.
/// `EPERM` means the process exists but belongs to another user — still
/// alive. `ESRCH` (the only other expected error) means gone.
pub fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: `kill(pid, 0)` is a standard POSIX probe that sends no
        // signal. The only side-effect is setting `errno`.
        let ret = unsafe { libc::kill(pid as i32, 0) };
        if ret == 0 {
            return true;
        }
        // EPERM → process exists, we just can't signal it.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        // On non-Unix we cannot cheaply check; assume alive.
        true
    }
}

/// Sends `SIGTERM` to `pid`.
///
/// Returns `Ok(true)` if the signal was delivered, `Ok(false)` if the
/// process was already gone (`ESRCH`), and `Err` on unexpected failures.
pub fn terminate(pid: u32) -> Result<bool> {
    #[cfg(unix)]
    {
        // SAFETY: Sending SIGTERM is a standard process control operation.
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret == 0 {
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        Err(err.into())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Ok(true)
    }
}

/// Sends `SIGTERM`, waits up to `timeout` for the process to exit, then
/// escalates to `SIGKILL` if it is still running.
pub fn terminate_gracefully(pid: u32, timeout: Duration) -> Result<()> {
    if !terminate(pid)? {
        return Ok(()); // Already gone.
    }

    let poll_interval = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + timeout;

    while std::time::Instant::now() < deadline {
        if !is_alive(pid) {
            return Ok(());
        }
        std::thread::sleep(poll_interval);
    }

    // Still alive after timeout — escalate.
    #[cfg(unix)]
    {
        // SAFETY: Sending SIGKILL is a standard process control operation.
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(is_alive(std::process::id()));
    }

    #[test]
    fn nonexistent_pid_is_not_alive() {
        // PID 4_000_000 is almost certainly not a real process.
        assert!(!is_alive(4_000_000));
    }

    #[test]
    fn terminate_nonexistent_returns_false() {
        assert!(!terminate(4_000_000).unwrap());
    }
}
