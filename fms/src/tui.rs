//! Full-viewport TUI for FMS (Ratatui + Crossterm).
//! Activated only when stdout is attached to a real terminal.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Frame, Terminal,
};

use crate::monitor::{self, MonitorEvent};
use crate::process;
use crate::stats::CpuTimes;
use crate::ui::{BillingMode, SessionParams};

// ── Internal channel messages ─────────────────────────────────────────────────

enum Msg {
    Tick { cpu_used: Duration, cpu_quota: Duration, mem_kb: u64, mem_limit_kb: Option<u64> },
    Done(MonitorEvent, Duration),
}

// ── Log ───────────────────────────────────────────────────────────────────────

const MAX_LOG: usize = 300;

#[derive(Clone, Copy)]
enum Level { Info, Success, Warn, Error, Metric }

#[derive(Clone)]
struct Entry { ts: String, level: Level, text: String }

fn now_ts() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{:02}:{:02}:{:02}", (s % 86400) / 3600, (s % 3600) / 60, s % 60)
}

// ── App state ─────────────────────────────────────────────────────────────────

struct App {
    params: SessionParams,
    quota_left: Duration,
    total_cpu: Duration,
    runs: u32,
    live_cpu: Duration,
    live_mem_kb: u64,
    tick_quota: Option<Duration>,
    tick_mem_limit: Option<u64>,
    input: String,
    is_running: bool,
    cancel: Option<Arc<AtomicBool>>,
    log: Vec<Entry>,
    should_quit: bool,
    exit_reason: String,
}

impl App {
    fn new(params: SessionParams) -> Self {
        let quota_left = params.cpu_quota;
        let mut a = Self {
            params,
            quota_left,
            total_cpu: Duration::ZERO,
            runs: 0,
            live_cpu: Duration::ZERO,
            live_mem_kb: 0,
            tick_quota: None,
            tick_mem_limit: None,
            input: String::new(),
            is_running: false,
            cancel: None,
            log: Vec::new(),
            should_quit: false,
            exit_reason: String::from("user quit"),
        };
        a.push(Level::Info, "FMS ready — type a binary path and press Enter to run.");
        match a.params.mode {
            BillingMode::Prepaid => a.push(Level::Info, &format!(
                "Prepaid mode — CPU budget {:.3}s", a.quota_left.as_secs_f64()
            )),
            BillingMode::Postpaid => a.push(Level::Info, "Postpaid mode — billed at session end."),
        }
        a
    }

    fn push(&mut self, level: Level, text: &str) {
        self.log.push(Entry { ts: now_ts(), level, text: text.to_string() });
        if self.log.len() > MAX_LOG {
            self.log.drain(0..self.log.len() - MAX_LOG);
        }
    }

    fn kill(&mut self) {
        if let Some(c) = &self.cancel { c.store(true, Ordering::Relaxed); }
    }

    fn launch(&mut self, tx: mpsc::Sender<Msg>) -> Result<(), String> {
        let raw = self.input.trim().to_string();
        if raw.is_empty() { return Ok(()); }
        let parts = shlex::split(&raw).unwrap_or_else(|| vec![raw.clone()]);
        if parts.is_empty() { return Ok(()); }

        let binary = std::path::PathBuf::from(&parts[0]);
        let args: Vec<String> = parts[1..].to_vec();

        let mut child = process::spawn(&binary, &args).map_err(|e| e.to_string())?;
        let child_pid = child.id();

        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancel = Some(Arc::clone(&cancelled));

        let live_cpu = Arc::new(Mutex::new(CpuTimes { user: Duration::ZERO, sys: Duration::ZERO }));
        let limits = Arc::new(monitor::Limits {
            timeout: self.params.timeout,
            mem_limit_kb: self.params.mem_limit_kb,
        });

        let progress: Arc<monitor::ProgressCallback> = Arc::new({
            let tx = tx.clone();
            move |cpu: Duration, quota: Duration, mem: u64, lim: Option<u64>| {
                let _ = tx.send(Msg::Tick { cpu_used: cpu, cpu_quota: quota, mem_kb: mem, mem_limit_kb: lim });
            }
        });

        let (_, rx) = monitor::start(
            child_pid, limits, self.quota_left,
            Arc::clone(&cancelled), Arc::clone(&live_cpu), progress,
        );

        std::thread::spawn(move || {
            let _ = child.wait();
            if let Ok(event) = rx.recv() {
                let run_cpu = match &event {
                    MonitorEvent::Exited { cpu, .. } | MonitorEvent::KilledUser { cpu, .. } => cpu.user + cpu.sys,
                    MonitorEvent::KilledTimeout | MonitorEvent::KilledMemory { .. } => {
                        live_cpu.lock().map(|c| c.user + c.sys).unwrap_or_default()
                    }
                };
                let _ = tx.send(Msg::Done(event, run_cpu));
            }
        });

        self.is_running = true;
        Ok(())
    }

    fn on_tick(&mut self, cpu_used: Duration, cpu_quota: Duration, mem_kb: u64, mem_limit_kb: Option<u64>) {
        self.live_cpu = cpu_used;
        self.live_mem_kb = mem_kb;
        self.tick_quota = (cpu_quota < Duration::MAX).then_some(cpu_quota);
        self.tick_mem_limit = mem_limit_kb;
        let pct = self.tick_quota.map(|q| {
            if q.is_zero() { 0.0 } else { (cpu_used.as_secs_f64() / q.as_secs_f64() * 100.0).min(100.0) }
        }).unwrap_or(0.0);
        self.push(Level::Metric, &format!(
            "CPU {:.3}s ({:.1}%)  RAM {:.1} MB", cpu_used.as_secs_f64(), pct, mem_kb as f64 / 1024.0
        ));
    }

    fn on_done(&mut self, event: MonitorEvent, run_cpu: Duration) {
        self.is_running = false;
        self.cancel = None;
        self.live_cpu = Duration::ZERO;
        self.live_mem_kb = 0;
        self.runs += 1;
        self.total_cpu += run_cpu;
        if self.params.mode == BillingMode::Prepaid {
            self.quota_left = self.quota_left.saturating_sub(run_cpu);
        }
        match event {
            MonitorEvent::Exited { cpu, peak_mem_kb } => self.push(Level::Success, &format!(
                "Done — CPU {:.3}s (usr {:.3}s + sys {:.3}s) | Peak RAM {:.1} MB",
                (cpu.user + cpu.sys).as_secs_f64(), cpu.user.as_secs_f64(),
                cpu.sys.as_secs_f64(), peak_mem_kb as f64 / 1024.0
            )),
            MonitorEvent::KilledUser { cpu, peak_mem_kb } => self.push(Level::Warn, &format!(
                "Killed by user — CPU {:.3}s | Peak RAM {:.1} MB",
                (cpu.user + cpu.sys).as_secs_f64(), peak_mem_kb as f64 / 1024.0
            )),
            MonitorEvent::KilledTimeout => self.push(Level::Warn, "Terminated — wall-clock timeout."),
            MonitorEvent::KilledMemory { peak_mem_kb } => {
                self.push(Level::Error, &format!(
                    "Terminated — memory limit exceeded (peak {:.1} MB).", peak_mem_kb as f64 / 1024.0
                ));
                self.exit_reason = "memory limit exceeded".into();
                self.should_quit = true;
                return;
            }
        }
        if self.params.mode == BillingMode::Prepaid {
            if self.quota_left.is_zero() {
                self.push(Level::Warn, "CPU quota exhausted.");
                self.exit_reason = "CPU quota exhausted".into();
                self.should_quit = true;
                return;
            }
            self.push(Level::Info, &format!("Quota remaining: {:.3}s", self.quota_left.as_secs_f64()));
        }
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(3)])
        .split(f.area());

    // ── Header ────────────────────────────────────────────────────────────────
    let mode_txt = match app.params.mode {
        BillingMode::Prepaid => format!(
            " Prepaid | Quota left: {:.3}s | Runs: {} | Total CPU: {:.3}s ",
            app.quota_left.as_secs_f64(), app.runs, app.total_cpu.as_secs_f64()
        ),
        BillingMode::Postpaid => format!(
            " Postpaid | Runs: {} | Total CPU: {:.3}s ",
            app.runs, app.total_cpu.as_secs_f64()
        ),
    };
    let status = if app.is_running {
        Span::styled(" ● RUNNING ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" ○ IDLE ", Style::default().fg(Color::Black).bg(Color::DarkGray))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("⚡ FMS", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  "), status, Span::raw("  "),
            Span::styled(&mode_txt, Style::default().fg(Color::White)),
        ])).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow))),
        root[0],
    );

    // ── Main split: metrics | log ─────────────────────────────────────────────
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(root[1]);

    // ── Metrics panel ─────────────────────────────────────────────────────────
    let metrics_block = Block::default().borders(Borders::ALL)
        .title(" Live Metrics ").border_style(Style::default().fg(Color::Cyan));
    let metrics_inner = metrics_block.inner(cols[0]);
    f.render_widget(metrics_block, cols[0]);

    let metric_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Length(1), Constraint::Length(3), Constraint::Min(0)])
        .split(metrics_inner);

    let cpu_pct = app.tick_quota.map(|q| {
        if q.is_zero() { 0 } else { ((app.live_cpu.as_secs_f64() / q.as_secs_f64()) * 100.0).min(100.0) as u16 }
    }).unwrap_or(0);
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" CPU "))
            .gauge_style(Style::default().fg(Color::Yellow).bg(Color::DarkGray))
            .percent(cpu_pct)
            .label(format!("{:.3}s{}",
                app.live_cpu.as_secs_f64(),
                app.tick_quota.map(|q| format!(" / {:.3}s", q.as_secs_f64())).unwrap_or_default()
            )),
        metric_chunks[0],
    );

    let mem_pct = app.tick_mem_limit.map(|lim| {
        ((app.live_mem_kb as f64 / lim as f64) * 100.0).min(100.0) as u16
    }).unwrap_or(0);
    f.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title(" RAM "))
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
            .percent(mem_pct)
            .label(format!("{:.1} MB{}",
                app.live_mem_kb as f64 / 1024.0,
                app.tick_mem_limit.map(|l| format!(" / {:.1} MB", l as f64 / 1024.0)).unwrap_or_default()
            )),
        metric_chunks[2],
    );

    // ── Log panel ─────────────────────────────────────────────────────────────
    let log_block = Block::default().borders(Borders::ALL)
        .title(" Session Log ").border_style(Style::default().fg(Color::Blue));
    let log_inner = log_block.inner(cols[1]);
    f.render_widget(log_block, cols[1]);

    let height = log_inner.height as usize;
    let items: Vec<ListItem> = app.log.iter().rev().take(height).rev()
        .map(|e| {
            let (sym, col) = match e.level {
                Level::Info    => ("●", Color::Cyan),
                Level::Success => ("✔", Color::Green),
                Level::Warn    => ("⚠", Color::Yellow),
                Level::Error   => ("✘", Color::Red),
                Level::Metric  => ("›", Color::DarkGray),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", e.ts), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{sym} "), Style::default().fg(col).add_modifier(Modifier::BOLD)),
                Span::styled(e.text.clone(), Style::default().fg(Color::White)),
            ]))
        })
        .collect();
    f.render_widget(List::new(items), log_inner);

    // ── Input bar ─────────────────────────────────────────────────────────────
    let hints = if app.is_running { "  [k] Kill  [q] Quit" } else { "  [↵] Run  [q] Quit" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" ❯ {}▌", app.input), Style::default().fg(Color::White)),
            Span::styled(hints, Style::default().fg(Color::DarkGray)),
        ])).block(Block::default().borders(Borders::ALL)
            .title(if app.is_running { " Running… " } else { " Command " })
            .border_style(if app.is_running {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            })),
        root[2],
    );
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(params: SessionParams) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let (tx, rx) = mpsc::channel::<Msg>();
    let mut app = App::new(params);

    let run_result: Result<(), Box<dyn std::error::Error>> = (|| loop {
        terminal.draw(|f| ui(f, &app))?;

        // Drain all pending monitor messages
        loop {
            match rx.try_recv() {
                Ok(Msg::Tick { cpu_used, cpu_quota, mem_kb, mem_limit_kb }) =>
                    app.on_tick(cpu_used, cpu_quota, mem_kb, mem_limit_kb),
                Ok(Msg::Done(event, run_cpu)) => app.on_done(event, run_cpu),
                Err(_) => break,
            }
        }

        if app.should_quit { break Ok(()); }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => { app.kill(); break Ok(()); }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.kill(); break Ok(());
                    }
                    KeyCode::Char('k') if app.is_running => {
                        app.push(Level::Warn, "Kill signal sent…");
                        app.kill();
                    }
                    KeyCode::Enter if !app.is_running => {
                        let cmd = app.input.clone();
                        match app.launch(tx.clone()) {
                            Ok(()) => app.push(Level::Info, &format!("Launched: {cmd}")),
                            Err(e) => app.push(Level::Error, &format!("Launch failed: {e}")),
                        }
                        app.input.clear();
                    }
                    KeyCode::Char(c) if !app.is_running => app.input.push(c),
                    KeyCode::Backspace if !app.is_running => { app.input.pop(); }
                    _ => {}
                }
            }
        }
    })();

    // Always restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Print final report in restored terminal
    println!("\n=== FMS Session Report ===");
    println!("  Runs      : {}", app.runs);
    println!("  Total CPU : {:.3}s", app.total_cpu.as_secs_f64());
    if app.params.mode == BillingMode::Postpaid {
        println!("  [POSTPAID] Bill: {:.3} CPU-seconds", app.total_cpu.as_secs_f64());
    }
    println!("  Exit      : {}", app.exit_reason);
    println!("==========================\n");

    run_result
}
