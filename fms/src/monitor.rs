use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::process::kill_tree;
use crate::stats;

pub struct Limits {
    pub timeout: Option<Duration>,
    pub mem_limit_kb: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub enum MonitorEvent {
    Exited {
        cpu: stats::CpuTimes,
        peak_mem_kb: u64,
    },
    KilledUser {
        cpu: stats::CpuTimes,
        peak_mem_kb: u64,
    },
    KilledTimeout,
    KilledMemory {
        peak_mem_kb: u64,
    },
}

pub type ProgressCallback = dyn Fn(Duration, Duration, u64, Option<u64>) + Send + Sync;

/// Starts the monitor thread. Returns a handle and a receiver for the final event.
pub fn start(
    child_pid: u32,
    limits: Arc<Limits>,
    cpu_quota: Duration,
    cancelled: Arc<AtomicBool>,
    live_cpu: Arc<Mutex<stats::CpuTimes>>,
    progress: Arc<ProgressCallback>,
) -> (
    thread::JoinHandle<MonitorEvent>,
    std::sync::mpsc::Receiver<MonitorEvent>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = thread::spawn(move || {
        let event = run(child_pid, &limits, cpu_quota, &cancelled, &live_cpu, &progress);
        let _ = tx.send(event);
        event
    });

    (handle, rx)
}

fn run(
    child_pid: u32,
    limits: &Limits,
    cpu_quota: Duration,
    cancelled: &AtomicBool,
    live_cpu: &Mutex<stats::CpuTimes>,
    progress: &Arc<ProgressCallback>,
) -> MonitorEvent {
    let start_wall = Instant::now();
    let poll = Duration::from_millis(100);
    let mut peak_mem_kb: u64 = 0;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            // Read final CPU *before* killing so the process is still alive
            let cpu = stats::tree_cpu_times(child_pid);
            let mem_kb = stats::tree_mem_kb(child_pid);
            if mem_kb > peak_mem_kb { peak_mem_kb = mem_kb; }
            *live_cpu.lock().expect("Live CPU mutex poisoned") = cpu;
            kill_tree(child_pid);
            return MonitorEvent::KilledUser { cpu, peak_mem_kb };
        }

        if !proc_exists(child_pid) {
            let cpu = *live_cpu.lock().expect("Live CPU mutex poisoned");
            return MonitorEvent::Exited { cpu, peak_mem_kb };
        }

        let cpu = stats::tree_cpu_times(child_pid);
        let cpu_total = cpu.user + cpu.sys;
        let mem_kb = stats::tree_mem_kb(child_pid);
        if mem_kb > peak_mem_kb {
            peak_mem_kb = mem_kb;
        }

        *live_cpu.lock().expect("Live CPU mutex poisoned") = cpu;

        progress(cpu_total, cpu_quota, mem_kb, limits.mem_limit_kb);

        if let Some(tl) = limits.timeout
            && start_wall.elapsed() >= tl {
                kill_tree(child_pid);
                return MonitorEvent::KilledTimeout;
            }

        if let Some(mem_lim) = limits.mem_limit_kb
            && mem_kb > mem_lim {
                kill_tree(child_pid);
                return MonitorEvent::KilledMemory { peak_mem_kb };
            }

        if cpu_quota < Duration::MAX && cpu_total >= cpu_quota {
            kill_tree(child_pid);
            return MonitorEvent::Exited { cpu, peak_mem_kb };
        }

        thread::sleep(poll);
    }
}

#[cfg(target_os = "linux")]
fn proc_exists(pid: u32) -> bool {
    let proc_dir = std::env::var("FMS_PROC_DIR").unwrap_or_else(|_| "/proc".to_string());
    std::fs::metadata(format!("{}/{}", proc_dir, pid)).is_ok()
}

#[cfg(target_os = "macos")]
fn proc_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn proc_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if handle == 0 { return false; }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == 259 // STILL_ACTIVE
    }
}
