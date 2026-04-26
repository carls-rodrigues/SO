use console::Term;
use fms::monitor;
use fms::process;
use fms::stats::CpuTimes;
use fms::tui;
use fms::ui::{self, BillingMode};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let params = ui::prompt_session_params();

    // If we are connected to a real terminal, hand off to the Ratatui TUI.
    if Term::stdout().is_term() {
        return tui::run(params);
    }

    // ── Plain-text fallback (used by automated tests & piped invocations) ──────
    let mut cpu_quota_remaining = params.cpu_quota;
    let mut total_cpu_used = Duration::ZERO;
    let mut total_runs = 0u32;

    let limits = Arc::new(monitor::Limits {
        timeout: params.timeout,
        mem_limit_kb: params.mem_limit_kb,
    });

    let exit_reason = loop {
        let Some((binary, args)) = ui::prompt_binary() else {
            break "user quit";
        };

        let mut child = match process::spawn(&binary, &args) {
            Ok(c) => c,
            Err(e) => {
                ui::print_launch_error(&e);
                continue;
            }
        };

        let child_pid = child.id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let live_cpu: Arc<Mutex<CpuTimes>> = Arc::new(Mutex::new(CpuTimes {
            user: Duration::ZERO,
            sys: Duration::ZERO,
        }));
        let progress = Arc::new(ui::LiveProgress::new());

        let (_handle, rx) = monitor::start(
            child_pid,
            Arc::clone(&limits),
            cpu_quota_remaining,
            Arc::clone(&cancelled),
            Arc::clone(&live_cpu),
            Arc::new({
                let p = Arc::clone(&progress);
                move |cpu, quota, mem, lim| p.update(cpu, quota, mem, lim)
            }),
        );

        // Wait for the child. Do NOT set cancelled — let the monitor detect
        // proc_exists = false so natural exits map to MonitorEvent::Exited.
        let _ = child.wait();

        let event = rx.recv().expect("monitor thread dropped channel");
        progress.finish();

        let run_cpu = match &event {
            monitor::MonitorEvent::Exited { cpu, .. }
            | monitor::MonitorEvent::KilledUser { cpu, .. } => cpu.user + cpu.sys,
            monitor::MonitorEvent::KilledTimeout | monitor::MonitorEvent::KilledMemory { .. } => {
                let cpu = *live_cpu.lock().unwrap();
                cpu.user + cpu.sys
            }
        };

        total_cpu_used += run_cpu;
        total_runs += 1;

        if params.mode == BillingMode::Prepaid {
            cpu_quota_remaining = cpu_quota_remaining.saturating_sub(run_cpu);
        }

        ui::print_run_summary(&event, cpu_quota_remaining);

        if matches!(&event, monitor::MonitorEvent::KilledMemory { .. }) {
            break "memory limit exceeded";
        }
        if params.mode == BillingMode::Prepaid && cpu_quota_remaining.is_zero() {
            break "CPU quota exhausted";
        }
    };

    ui::print_final_report(total_cpu_used, total_runs, exit_reason, params.mode);
    Ok(())
}
