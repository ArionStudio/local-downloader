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
