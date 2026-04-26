use tauri::{AppHandle, Emitter, State};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Shared session state ──────────────────────────────────────────────────────

/// Holds the active session so `kill_fms_session` can reach it from any thread.
struct ActiveSession {
    child_pid: u32,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct SessionState(Mutex<Option<ActiveSession>>);

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SessionReq {
    pub binary: String,
    pub args: Vec<String>,
    pub cpu_quota_secs: Option<f64>,
    pub timeout_secs: Option<f64>,
    pub mem_limit_kb: Option<u64>,
}

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    cpu_used: f64,
    cpu_quota: Option<f64>,
    mem_kb: u64,
    mem_limit_kb: Option<u64>,
}

#[derive(Clone, serde::Serialize)]
struct RunSummary {
    status: &'static str,
    cpu_user: f64,
    cpu_sys: f64,
    peak_mem_kb: u64,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn spawn_fms_session(
    app_handle: AppHandle,
    session_state: State<'_, SessionState>,
    req: SessionReq,
) -> Result<RunSummary, String> {
    let cpu_quota = req
        .cpu_quota_secs
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::MAX);

    let limits = Arc::new(fms::monitor::Limits {
        timeout: req.timeout_secs.map(Duration::from_secs_f64),
        mem_limit_kb: req.mem_limit_kb,
    });

    let mut child =
        fms::process::spawn(&std::path::PathBuf::from(req.binary), &req.args)
            .map_err(|e| e.to_string())?;

    let child_pid = child.id();
    let cancelled = Arc::new(AtomicBool::new(false));
    let live_cpu = Arc::new(Mutex::new(fms::stats::CpuTimes {
        user: Duration::ZERO,
        sys: Duration::ZERO,
    }));

    // Register the session so kill_fms_session can reach it
    {
        let mut guard = session_state.0.lock().unwrap();
        *guard = Some(ActiveSession {
            child_pid,
            cancelled: Arc::clone(&cancelled),
        });
    }

    let progress = Arc::new({
        let app = app_handle.clone();
        move |cpu: Duration, quota: Duration, mem: u64, lim: Option<u64>| {
            let _ = app.emit(
                "fms-progress",
                ProgressPayload {
                    cpu_used: cpu.as_secs_f64(),
                    cpu_quota: if quota < Duration::MAX {
                        Some(quota.as_secs_f64())
                    } else {
                        None
                    },
                    mem_kb: mem,
                    mem_limit_kb: lim,
                },
            );
        }
    });

    let (_handle, rx) = fms::monitor::start(
        child_pid,
        limits,
        cpu_quota,
        Arc::clone(&cancelled),
        Arc::clone(&live_cpu),
        progress,
    );

    // Reap the direct child in a background thread so a daemonising process
    // (e.g. VLC forking itself) does not prematurely end our monitor loop.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    // Block until the monitor declares the session over (timeout / OOM / natural exit).
    let event = rx.recv().map_err(|e| e.to_string())?;

    // Clear the session slot *after* the event, so kill_fms_session still works
    // during the wait above.
    *session_state.0.lock().unwrap() = None;

    let (status, cpu_total, peak_mem) = match event {
        fms::monitor::MonitorEvent::Exited { cpu, peak_mem_kb } => ("Exited", cpu, peak_mem_kb),
        fms::monitor::MonitorEvent::KilledUser { cpu, peak_mem_kb } => ("KilledUser", cpu, peak_mem_kb),
        fms::monitor::MonitorEvent::KilledTimeout => {
            let cpu = *live_cpu.lock().unwrap();
            ("KilledTimeout", cpu, 0)
        }
        fms::monitor::MonitorEvent::KilledMemory { peak_mem_kb } => {
            let cpu = *live_cpu.lock().unwrap();
            ("KilledMemory", cpu, peak_mem_kb)
        }
    };

    Ok(RunSummary {
        status,
        cpu_user: cpu_total.user.as_secs_f64(),
        cpu_sys: cpu_total.sys.as_secs_f64(),
        peak_mem_kb: peak_mem,
    })
}

/// Signals the monitor to stop and kills the process tree via the monitor thread.
/// We only set the flag here — the monitor does the final CPU read then kills.
#[tauri::command]
fn kill_fms_session(session_state: State<'_, SessionState>) -> Result<(), String> {
    let guard = session_state.0.lock().unwrap();
    match guard.as_ref() {
        Some(s) => {
            s.cancelled.store(true, Ordering::Relaxed);
            Ok(())
        }
        None => Err("No active session to kill.".into()),
    }
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SessionState::default())
        .invoke_handler(tauri::generate_handler![spawn_fms_session, kill_fms_session])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
