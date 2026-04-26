use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use crate::stats;

pub fn spawn(binary: &PathBuf, args: &[String]) -> std::io::Result<Child> {
    Command::new(binary)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

/// Sends SIGKILL to root_pid and every descendant.
pub fn kill_tree(root_pid: u32) {
    let mut pids = stats::collect_descendants(root_pid);
    pids.push(root_pid);
    kill_pids(&pids);
}

#[cfg(unix)]
fn kill_pids(pids: &[u32]) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    for &pid in pids {
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_pids(pids: &[u32]) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    for &pid in pids {
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle != 0 {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}
