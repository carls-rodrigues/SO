use std::process::{Command, Stdio};
use std::io::Write;

fn run_fms_with_input(input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fms"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).unwrap();
    drop(stdin); // closes stdin

    child.wait_with_output().unwrap()
}

#[test]
fn test_cli_basic_prepaid_run() {
    // 1 -> Prepaid
    // 0.5 -> CPU quota
    // \n -> timeout length (blank)
    // \n -> mem limit (blank)
    // sleep 0.1 -> run cmd
    // quit -> exit
    let input = "1\n0.5\n\n\nsleep 0.1\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remaining CPU quota"));
}

#[test]
fn test_cli_postpaid_run() {
    // 2 -> Postpaid
    // \n -> timeout blank
    // \n -> mem blank
    // sleep 0.1 -> cmd
    // quit -> exit
    let input = "2\n\n\nsleep 0.1\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[POSTPAID] You owe"));
}

#[test]
fn test_cli_invalid_inputs() {
    // 3 -> Invalid mode (should ask again)
    // 1 -> Prepaid
    // invalid -> Invalid CPU quota
    // -5 -> Invalid positive CPU quota
    // 1.0 -> Valid CPU quota
    // abc -> Invalid timeout
    // -2.0 -> Invalid negative timeout
    // \n -> Blank timeout
    // xyz -> Invalid mem limit
    // \n -> Blank mem limit
    // quit -> exit
    let input = "3\n1\ninvalid\n-5\n1.0\nabc\n-2.0\n\nxyz\n\nquit\n";
    let output = run_fms_with_input(input);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Please enter 1 or 2"));
    assert!(stderr.contains("Please enter a valid positive number"));
    assert!(stderr.contains("Please enter a positive integer"));
}

#[test]
fn test_cli_launch_error() {
    let input = "1\n10.0\n\n\n/non/existent/binary\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed to launch binary"));
}

#[test]
fn test_cli_exhausted_quota() {
    // Prepaid, 0.1s CPU quota
    // yes command consumes 100% CPU loops endlessly
    let input = "1\n0.1\n\n\nyes\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CPU quota exhausted"));
}

#[test]
fn test_cli_memory_limit_exceeded() {
    // 1 -> Prepaid
    // 10.0 -> CPU
    // \n -> Blank timeout
    // 10 -> 10MB mem limit
    std::fs::write("/tmp/fms_test_alloc.py", "import time\na=[0]*5000000\ntime.sleep(10)\n").unwrap();
    let input = "1\n10.0\n\n10\npython3 /tmp/fms_test_alloc.py\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("memory limit exceeded"));
}

#[test]
fn test_cli_timeout_exceeded() {
    // 1 -> Prepaid
    // 10.0 -> CPU
    // 1.0 -> Timeout 1 second
    // \n -> limit blank
    // sleep 5
    let input = "1\n10.0\n1.0\n\nsleep 5\nquit\n";
    let output = run_fms_with_input(input);
    
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Wall-clock timeout expired"));
}
