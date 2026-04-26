use fms::monitor::{self, Limits, MonitorEvent};
use fms::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn test_monitor_timeout() {
    // Spawn a child process that sleeps for 5 seconds
    let mut child = process::spawn(
        &"/bin/sh".into(),
        &["-c".to_string(), "sleep 5".to_string()],
    )
    .unwrap();

    let pid = child.id();

    let limits = Arc::new(Limits {
        timeout: Some(Duration::from_secs(1)),
        mem_limit_kb: None,
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let live_cpu = Arc::new(Mutex::new(fms::stats::CpuTimes {
        user: Duration::ZERO,
        sys: Duration::ZERO,
    }));
    let progress = Arc::new(|_: Duration, _: Duration, _: u64, _: Option<u64>| {});

    let (handle, rx) = monitor::start(pid, limits, Duration::MAX, Arc::clone(&cancelled), live_cpu, progress);

    let event = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    
    match event {
        MonitorEvent::KilledTimeout => {
            // Expected
        }
        _ => panic!("Expected process to be killed by timeout, got {:?}", event),
    }

    handle.join().unwrap();
    let _ = child.wait();
}

#[test]
fn test_monitor_memory_limit() {
    // Spawn a script that allocates ~20MB of memory and holds it
    let mut child = process::spawn(
        &"python3".into(),
        &[
            "-c".to_string(),
            "a = [0] * 5000000; import time; time.sleep(10)".to_string(),
        ],
    )
    .unwrap();

    let pid = child.id();

    // Set memory limit to 5MB (5000 KB)
    let limits = Arc::new(Limits {
        timeout: Some(Duration::from_secs(10)),
        mem_limit_kb: Some(5000), 
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let live_cpu = Arc::new(Mutex::new(fms::stats::CpuTimes {
        user: Duration::ZERO,
        sys: Duration::ZERO,
    }));
    let progress = Arc::new(|_: Duration, _: Duration, _: u64, _: Option<u64>| {});

    let (handle, rx) = monitor::start(pid, limits, Duration::MAX, Arc::clone(&cancelled), live_cpu, progress);

    let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    
    match event {
        MonitorEvent::KilledMemory { peak_mem_kb } => {
            assert!(peak_mem_kb > 5000);
        }
        _ => panic!("Expected process to be killed by memory limit, got {:?}", event),
    }

    handle.join().unwrap();
    let _ = child.wait();
}

#[test]
fn test_monitor_exited_naturally() {
    // Spawn a process that exits very quickly
    let mut child = process::spawn(
        &"/bin/sh".into(),
        &["-c".to_string(), "exit 0".to_string()],
    )
    .unwrap();

    let pid = child.id();

    let limits = Arc::new(Limits {
        timeout: Some(Duration::from_secs(5)),
        mem_limit_kb: Some(50000),
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let live_cpu = Arc::new(Mutex::new(fms::stats::CpuTimes {
        user: Duration::ZERO,
        sys: Duration::ZERO,
    }));
    let progress = Arc::new(|_: Duration, _: Duration, _: u64, _: Option<u64>| {});

    let (handle, rx) = monitor::start(pid, limits, Duration::MAX, Arc::clone(&cancelled), live_cpu, progress);

    // Wait for it to exit naturally
    let _ = child.wait();
    // Allow monitor loop to poll and notice it died
    std::thread::sleep(Duration::from_millis(300));
    cancelled.store(true, Ordering::Relaxed);

    let event = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    
    match event {
        MonitorEvent::Exited { .. } => {
            // Expected
        }
        _ => panic!("Expected process to exit naturally, got {:?}", event),
    }

    handle.join().unwrap();
    let _ = child.wait();
}
