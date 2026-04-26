use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;
use console::{style, Term};

use crate::monitor::MonitorEvent;

pub struct SessionParams {
    pub cpu_quota: Duration,
    pub timeout: Option<Duration>,
    pub mem_limit_kb: Option<u64>,
    pub mode: BillingMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BillingMode {
    Prepaid,
    Postpaid,
}

pub fn prompt_session_params() -> SessionParams {
    println!("=== FMS — Process Manager ===");

    let mode = prompt_billing_mode();

    let cpu_quota = if mode == BillingMode::Prepaid {
        if !Term::stdout().is_term() {
            let secs = prompt_f64("CPU quota (seconds of CPU time): ", Some(0.001));
            Duration::from_secs_f64(secs)
        } else {
            let secs: f64 = dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt("CPU quota (seconds of CPU time)")
                .default(1.0)
                .validate_with(|inp: &f64| {
                    if *inp > 0.0 { Ok(()) } else { Err("Value must be > 0") }
                })
                .interact_text()
                .unwrap();
            Duration::from_secs_f64(secs)
        }
    } else {
        Duration::MAX
    };

    let (timeout_secs, mem_limit_kb) = if !Term::stdout().is_term() {
        let ts = prompt_optional_f64(
            "Wall-clock timeout per run (seconds, leave blank for none): ",
            Some(0.001),
        );
        let ml = prompt_optional_u64("Max memory per run (MB, leave blank for none): ")
            .map(|mb| mb * 1024);
        (ts, ml)
    } else {
        let ts: String = dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Wall-clock timeout per run (seconds) [leave blank for none]")
            .allow_empty(true)
            .interact_text()
            .unwrap();
        let timeout_secs = ts.parse::<f64>().ok().filter(|&x| x > 0.0);

        let ml: String = dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Max memory per run (MB) [leave blank for none]")
            .allow_empty(true)
            .interact_text()
            .unwrap();
        let mem_limit_kb = ml.parse::<u64>().ok().filter(|&x| x > 0).map(|mb| mb * 1024);
        (timeout_secs, mem_limit_kb)
    };

    SessionParams {
        cpu_quota,
        timeout: timeout_secs.map(Duration::from_secs_f64),
        mem_limit_kb,
        mode,
    }
}

pub fn prompt_binary() -> Option<(PathBuf, Vec<String>)> {
    loop {
        let line = if !Term::stdout().is_term() {
            prompt_line("\nBinary to run (path [args...], or 'quit' to exit): ")
        } else {
            println!();
            dialoguer::Input::<String>::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt("Binary to run (path [args...], or 'quit')")
                .interact_text()
                .unwrap_or_else(|_| "quit".into())
        };

        if line.is_empty() || line == "quit" {
            return None;
        }

        let Some(tokens) = shlex::split(&line) else {
            eprintln!("  Invalid input: unmatched quote.");
            continue;
        };

        let mut parts = tokens.into_iter();
        if let Some(bin) = parts.next() {
            let binary = PathBuf::from(bin);
            let args: Vec<String> = parts.collect();
            return Some((binary, args));
        }
    }
}

pub fn print_launch_error(err: &std::io::Error) {
    eprintln!("{}", style(format!("[FMS] Failed to launch binary: {err}")).red().bold());
}

pub struct LiveProgress {
    pb: Option<indicatif::ProgressBar>,
}

impl LiveProgress {
    pub fn new() -> Self {
        if !Term::stdout().is_term() {
            return Self { pb: None };
        }
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::default_spinner()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠉ ")
                .template("{spinner:.cyan.bold} {msg}")
                .unwrap(),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        Self { pb: Some(pb) }
    }

    pub fn update(&self, cpu_used: Duration, cpu_quota: Duration, mem_kb: u64, mem_limit: Option<u64>) {
        let mem_mb = mem_kb as f64 / 1024.0;
        let cpu_used_secs = cpu_used.as_secs_f64();
        let cpu_display = if cpu_quota < Duration::MAX {
            format!("{cpu_used_secs:.3}s / {:.3}s CPU", cpu_quota.as_secs_f64())
        } else {
            format!("{cpu_used_secs:.3}s CPU")
        };
        let mem_display = match mem_limit {
            Some(lim) => format!("{mem_mb:.1} MB / {:.1} MB RAM", lim as f64 / 1024.0),
            None => format!("{mem_mb:.1} MB RAM"),
        };

        let msg = format!("[{}] {}  |  {}", style("RUNNING").cyan(), cpu_display, mem_display);

        if let Some(pb) = &self.pb {
            pb.set_message(msg);
        } else {
            print!("\r  {msg}    ");
            let _ = io::stdout().flush();
        }
    }

    pub fn finish(&self) {
        if let Some(pb) = &self.pb {
            pb.finish_and_clear();
        } else {
            println!();
        }
    }
}

pub fn print_run_summary(event: &MonitorEvent, quota_remaining: Duration) {
    match event {
        MonitorEvent::Exited {
            cpu,
            peak_mem_kb,
        } => {
            println!(
                "  {} CPU user={:.3}s sys={:.3}s total={:.3}s | peak RAM={:.1} MB",
                style("[DONE]").green().bold(),
                cpu.user.as_secs_f64(),
                cpu.sys.as_secs_f64(),
                (cpu.user + cpu.sys).as_secs_f64(),
                *peak_mem_kb as f64 / 1024.0
            );
        }
        MonitorEvent::KilledTimeout => {
            println!("  {} Wall-clock timeout expired.", style("[KILLED]").red().bold());
        }
        MonitorEvent::KilledMemory { peak_mem_kb } => {
            println!(
                "  {} Memory limit exceeded (peak {:.1} MB).",
                style("[KILLED]").red().bold(),
                *peak_mem_kb as f64 / 1024.0
            );
        }
        MonitorEvent::KilledUser { cpu, peak_mem_kb } => {
            println!(
                "  {} Killed by user. CPU user={:.3}s sys={:.3}s | peak RAM={:.1} MB",
                style("[KILLED]").red().bold(),
                cpu.user.as_secs_f64(),
                cpu.sys.as_secs_f64(),
                *peak_mem_kb as f64 / 1024.0
            );
        }
    }
    if quota_remaining < Duration::MAX {
        println!("  Remaining CPU quota: {}", style(format!("{:.3}s", quota_remaining.as_secs_f64())).cyan());
    }
}

pub fn print_final_report(total_cpu: Duration, total_runs: u32, reason: &str, mode: BillingMode) {
    println!("\n{}", style("=== FMS Session Report ===").bold());
    println!("  Total runs completed : {}", style(total_runs).cyan());
    println!("  Total CPU consumed   : {}s", style(format!("{:.3}", total_cpu.as_secs_f64())).cyan());
    if mode == BillingMode::Postpaid {
        println!("  {} You owe {} CPU-seconds.", style("[POSTPAID]").yellow().bold(), style(format!("{:.3}", total_cpu.as_secs_f64())).magenta());
    }
    println!("  Exit reason          : {}", if reason.contains("user quit") { style(reason).green() } else { style(reason).red() });
    println!("{}", style("==========================").bold());
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn prompt_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    buf.trim().to_string()
}

fn prompt_f64(prompt: &str, min: Option<f64>) -> f64 {
    loop {
        let s = prompt_line(prompt);
        match s.parse::<f64>() {
            Ok(v) if min.is_none_or(|m| v >= m) => return v,
            _ => eprintln!("  Please enter a valid positive number."),
        }
    }
}

fn prompt_optional_f64(prompt: &str, min: Option<f64>) -> Option<f64> {
    loop {
        let s = prompt_line(prompt);
        if s.is_empty() {
            return None;
        }
        match s.parse::<f64>() {
            Ok(v) if min.is_none_or(|m| v >= m) => return Some(v),
            _ => eprintln!("  Please enter a valid positive number or leave blank."),
        }
    }
}

fn prompt_optional_u64(prompt: &str) -> Option<u64> {
    loop {
        let s = prompt_line(prompt);
        if s.is_empty() {
            return None;
        }
        match s.parse::<u64>() {
            Ok(v) if v > 0 => return Some(v),
            _ => eprintln!("  Please enter a positive integer or leave blank."),
        }
    }
}

fn prompt_billing_mode() -> BillingMode {
    if !Term::stdout().is_term() {
        loop {
            let s = prompt_line(
                "Billing mode — [1] Prepaid (set CPU budget upfront)  [2] Postpaid (pay per use): ",
            );
            match s.as_str() {
                "1" => return BillingMode::Prepaid,
                "2" => return BillingMode::Postpaid,
                _ => eprintln!("  Please enter 1 or 2."),
            }
        }
    }

    let items = &["Prepaid (set CPU budget upfront)", "Postpaid (pay per use)"];
    let selection = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Billing mode")
        .default(0)
        .items(&items[..])
        .interact()
        .unwrap();

    if selection == 0 {
        BillingMode::Prepaid
    } else {
        BillingMode::Postpaid
    }
}
