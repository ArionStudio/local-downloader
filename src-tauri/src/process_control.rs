use std::process::Command;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        command.process_group(0);
    }

    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

pub fn terminate_process_group(process_id: u32) {
    #[cfg(unix)]
    {
        signal_process_group(process_id, libc::SIGTERM);
    }

    #[cfg(not(unix))]
    {
        let _ = process_id;
    }
}

pub fn force_kill_process_group(process_id: u32) {
    #[cfg(unix)]
    {
        signal_process_group(process_id, libc::SIGKILL);
    }

    #[cfg(not(unix))]
    {
        let _ = process_id;
    }
}

#[cfg(unix)]
fn signal_process_group(process_id: u32, signal: libc::c_int) {
    if process_id > i32::MAX as u32 {
        return;
    }

    let process_group_id = -(process_id as libc::pid_t);
    unsafe {
        libc::kill(process_group_id, signal);
    }
}

/// Reap a subprocess on every exit path, including cancellation and errors.
pub struct ChildGuard(pub std::process::Child);
impl std::ops::Deref for ChildGuard {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            force_kill_process_group(self.0.id());
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
