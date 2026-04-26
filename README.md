# FMS — Process Management System

FMS is a cross-platform process manager written in Rust. You give it any compiled program and it launches that program for you, monitors it live, and enforces three independent resource limits: CPU quota, wall-clock timeout, and memory cap. It tracks the **entire process tree**, not just the root process.

## Requirements

- Rust 1.85+ (edition 2024)
- Linux, macOS, or Windows
- `python3` in `$PATH` (only needed for the memory-limit integration test)

## Building

```bash
cargo build --release
```

## Running

```bash
# Linux / macOS
./target/release/fms

# Windows
.\target\release\fms.exe

# or without building first
cargo run
```

FMS does **not** need the program you want to monitor to be running beforehand. You compile your target program once, then give its path to FMS — FMS is the one that launches it.

```
1. compile your program  →  produces an executable file on disk
2. run FMS
3. FMS asks: "Binary to run?"
4. you type the path to that executable
5. FMS launches it, monitors it, reports CPU and memory
6. your program runs and exits
7. FMS asks for the next binary
```

---

## Step-by-step setup

FMS asks four questions at startup. Here is what each one means and what to enter.

---

### 1 — Billing mode

```
Billing mode — [1] Prepaid  [2] Postpaid:
```

| Choice | What it does |
|--------|-------------|
| **1 — Prepaid** | You set a CPU budget upfront. Each run deducts from it. When the budget hits zero, FMS exits automatically. |
| **2 — Postpaid** | No budget cap. FMS accumulates total CPU across all runs and shows the total when you quit. |

**Choose Prepaid** when you want a hard limit — for example, to guarantee a demo stays within a fixed CPU budget.

**Choose Postpaid** when you just want to observe and measure, without any automatic cutoff.

---

### 2 — CPU quota *(prepaid only)*

```
CPU quota (seconds of CPU time): 10
```

The total CPU-seconds available across all runs in this session.

**CPU time is not the same as wall-clock time.** If a program runs for 4 real seconds but only uses 50% of one core, it consumed 2 CPU-seconds. A program that sleeps consumes almost no CPU at all.

| Value | Meaning |
|-------|---------|
| `5` | 5 seconds of actual CPU work across all runs |
| `0.5` | Half a second — useful for testing fast programs |
| `60` | One minute of CPU — for heavier workloads |

Decimals are accepted. Minimum is `0.001`.

---

### 3 — Wall-clock timeout

```
Wall-clock timeout per run (seconds) [leave blank for none]:
```

The maximum **real time** a single run may take. When it expires, the program is killed and FMS moves on to the next binary — **the session does not end**.

| Value | Meaning |
|-------|---------|
| `5` | Kill any program that runs longer than 5 real seconds |
| `30` | Allow up to 30 seconds of real time |
| *(blank)* | No timeout — program runs until it finishes or another limit kills it |

**Why use this?** A program can be sleeping, waiting on network, or stuck in I/O — consuming almost no CPU but taking forever. The CPU quota alone would never catch that. The timeout does.

---

### 4 — Memory limit

```
Max memory per run (MB) [leave blank for none]:
```

The peak RSS (resident memory) cap per run across the entire process tree. If exceeded, the program is killed and **the session ends**.

| Value | Meaning |
|-------|---------|
| `128` | Cap at 128 MB — reasonable for most CLI programs |
| `512` | Cap at 512 MB — for heavier workloads |
| *(blank)* | No memory limit |

**Why use this?** A runaway program or memory leak can exhaust all system RAM and crash the machine. The memory limit protects the host.

---

### 5 — Running a binary

```
Binary to run (path [args...], or 'quit'):
```

Enter the **full path** to any compiled executable, followed by any arguments it needs. FMS launches it, monitors it, and reports the result. Type `quit` or leave blank to end the session.

#### Your own Rust program

Build it first in its own directory, then give FMS the path to the compiled binary:

```bash
# in your other project's directory
cargo build --release
```

Then in FMS:

```
# Linux / macOS
Binary to run: /home/user/my_project/target/release/my_program

# Windows
Binary to run: C:\Users\user\my_project\target\release\my_program.exe
```

With arguments:

```
Binary to run: /home/user/my_project/target/release/my_program --flag value input.txt
```

FMS doesn't care what language the binary was written in. Any compiled executable works.

---

#### Quick programs — verify normal flow

Linux / macOS:
```
Binary to run: /usr/bin/ls -la /home
Binary to run: /usr/bin/date
Binary to run: /usr/bin/echo hello world
```

Windows:
```
Binary to run: C:\Windows\System32\whoami.exe
Binary to run: C:\Windows\System32\hostname.exe
Binary to run: C:\Windows\System32\cmd.exe /c dir C:\Users
```

---

#### CPU-intensive — test the quota

Linux / macOS:
```
Binary to run: /usr/bin/gzip -k /var/log/syslog
Binary to run: /usr/bin/sort /usr/share/dict/words
Binary to run: /usr/bin/find / -name "*.conf"
```

Windows:
```
Binary to run: C:\Windows\System32\cmd.exe /c dir /s C:\Windows
Binary to run: C:\Windows\System32\powershell.exe -Command "1..1000000 | Measure-Object -Sum"
```

---

#### Long-running / infinite — test the timeout

These never finish on their own. Set a wall-clock timeout and FMS will kill them after that many seconds, then continue to the next binary.

Linux / macOS:
```
Binary to run: /bin/sleep 30
Binary to run: /bin/sh -c "while true; do echo running; sleep 1; done"
```

Windows:
```
Binary to run: C:\Windows\System32\ping.exe -t 127.0.0.1
Binary to run: C:\Windows\System32\cmd.exe /c pause
```

---

#### Memory-intensive — test the memory limit

Set a memory limit (e.g. 50 MB) before running these. FMS kills the program the moment it exceeds the cap and ends the session.

Linux / macOS / Windows (requires Python):
```
Binary to run: python3 -c "a = [0] * 10000000; import time; time.sleep(5)"
```

Windows (PowerShell):
```
Binary to run: C:\Windows\System32\powershell.exe -Command "$a = New-Object byte[] 200MB; Start-Sleep 10"
```

---

#### Multi-process — test process-tree tracking

FMS monitors the entire process tree. All child processes are tracked together; their CPU and memory are summed.

Linux / macOS:
```
Binary to run: /bin/sh -c "sleep 5 & sleep 5 & wait"
```

This spawns a shell plus two `sleep` children. FMS reports the combined usage of all three.

Windows:
```
Binary to run: C:\Windows\System32\cmd.exe /c "start /b ping -t 127.0.0.1 & ping -t 127.0.0.1"
```

---

#### Discover available executables

```bash
# Linux / macOS
ls /usr/bin/

# Windows (PowerShell)
Get-Command * | Where-Object { $_.CommandType -eq "Application" } | Select-Object Name, Source
```

---

## Live output

While a program is running, FMS shows a spinner that updates every 100 ms:

```
⠇ [RUNNING] 1.243s / 10.000s CPU  |  45.2 MB / 128.0 MB RAM
```

- Left number: CPU time consumed so far in this run
- Right number: total quota remaining (prepaid only)
- RAM: current memory across the entire process tree

---

## After each run

#### Normal exit

```
  [DONE] CPU user=1.123s sys=0.034s total=1.157s | peak RAM=45.2 MB
  Remaining CPU quota: 8.843s
```

| Field | Meaning |
|-------|---------|
| **user** | Time the CPU spent executing your program's own code |
| **sys** | Time the CPU spent in kernel calls on behalf of your program (file I/O, memory allocation, etc.) |
| **total** | user + sys — the amount deducted from the prepaid quota |
| **peak RAM** | Maximum memory seen across the entire process tree during this run |

#### Killed by timeout

```
  [KILLED] Wall-clock timeout expired.
```

The program was running too long in real time. FMS killed it. The session continues — type the next binary.

#### Killed by memory limit

```
  [KILLED] Memory limit exceeded (peak 210.3 MB).
```

The process tree exceeded the memory cap. FMS killed it and the session ends.

---

## Session report

When the session ends for any reason:

```
=== FMS Session Report ===
  Total runs completed : 4
  Total CPU consumed   : 8.761s
  Exit reason          : CPU quota exhausted
==========================
```

In postpaid mode the bill is also shown:

```
  [POSTPAID] You owe 8.761 CPU-seconds.
```

---

## Termination rules

| Event | Session ends? | Why |
|-------|--------------|-----|
| Wall-clock timeout expires | **No** | Timeout is a per-run safety net. FMS continues and waits for the next binary. |
| CPU quota exhausted (prepaid) | **Yes** | The total budget is gone. |
| Memory limit exceeded | **Yes** | A runaway process is a system risk; the session stops to protect the host. |
| User types `quit` | **Yes** | Explicit exit. |

---

## Complete worked examples

### Example A — Running your own Rust program with a budget

> Goal: compile a separate Rust project and monitor it with a 10-second CPU budget and 5-second timeout.

```bash
# step 1: build your other project
cd ~/my_rust_project
cargo build --release

# step 2: run FMS
cd ~/fms
cargo run
```

```
Billing mode: 1
CPU quota: 10
Wall-clock timeout: 5
Memory limit: (blank)

Binary to run: /home/user/my_rust_project/target/release/my_program
  → [DONE] CPU user=0.841s sys=0.102s total=0.943s | peak RAM=12.3 MB
  Remaining CPU quota: 9.057s

Binary to run: quit

=== FMS Session Report ===
  Total runs completed : 1
  Total CPU consumed   : 0.943s
  Exit reason          : user quit
```

---

### Example B — Stress-testing with a strict budget

> Goal: run three programs with a total CPU budget of 5 seconds.

```
Billing mode: 1
CPU quota: 5
Wall-clock timeout: 3
Memory limit: (blank)

Binary to run: /usr/bin/gzip -k /var/log/syslog
  → [DONE] CPU total=1.203s | peak RAM=2.1 MB  (quota left: 3.797s)

Binary to run: /usr/bin/sort /usr/share/dict/words
  → [DONE] CPU total=2.441s | peak RAM=8.4 MB  (quota left: 1.356s)

Binary to run: /usr/bin/grep -r "error" /var/log
  → [DONE] CPU total=1.401s | peak RAM=3.2 MB  (quota left: 0s)

=== FMS Session Report ===
  Total runs completed : 3
  Total CPU consumed   : 5.045s
  Exit reason          : CPU quota exhausted
```

FMS exits automatically — no need to type `quit`.

---

### Example C — Catching an infinite loop

> Goal: demonstrate that a program that never exits gets killed by the timeout, and the session continues.

```
Billing mode: 2
Wall-clock timeout: 2
Memory limit: (blank)

Binary to run: /bin/sh -c "while true; do :; done"
  → [KILLED] Wall-clock timeout expired.
  (FMS continues — timeout does not end the session)

Binary to run: /usr/bin/date
  → [DONE] CPU total=0.002s | peak RAM=1.1 MB

Binary to run: quit

=== FMS Session Report ===
  Total runs completed : 2
  Total CPU consumed   : 2.003s
  [POSTPAID] You owe 2.003 CPU-seconds.
```

---

### Example D — Catching a memory leak

> Goal: a program allocates too much memory and gets killed.

```
Billing mode: 2
Wall-clock timeout: (blank)
Memory limit: 50

Binary to run: python3 -c "a = [0] * 20000000; import time; time.sleep(10)"
  → [KILLED] Memory limit exceeded (peak 156.4 MB).

=== FMS Session Report ===
  Total runs completed : 1
  Total CPU consumed   : 0.312s
  Exit reason          : memory limit exceeded
```

---

## Architecture

```
main.rs      session loop: collect params → spawn binary → wait → accounting → repeat
ui.rs        terminal I/O: dialoguer prompts, indicatif spinner, coloured summaries
monitor.rs   monitor thread: 100 ms poll loop; enforces timeout, memory, CPU quota
process.rs   spawn child (inheriting stdio); kill_tree (SIGKILL / TerminateProcess)
stats.rs     platform-specific process stats; platform-agnostic tree walker
lib.rs       re-exports the four modules
```

### Thread and IPC model

```
main thread                    monitor thread
-----------                    --------------
spawn child
start monitor ──────────────►  poll every 100 ms
child.wait()                   write live_cpu → Arc<Mutex<CpuTimes>>
cancelled = true ─────────────►detect cancelled flag
rx.recv() ◄────────────────── tx.send(MonitorEvent)
```

| Primitive | Purpose |
|-----------|---------|
| `mpsc::channel` | Monitor sends the final `MonitorEvent` to main |
| `Arc<AtomicBool>` (`cancelled`) | Main signals that the child has been reaped |
| `Arc<Mutex<CpuTimes>>` (`live_cpu`) | Monitor writes latest CPU reading; main reads it for killed events |

### Platform-specific stats

| Platform | CPU times | Memory | Process listing |
|----------|-----------|--------|-----------------|
| Linux | `/proc/<pid>/stat` (kernel ticks via `sysconf`) | `/proc/<pid>/status` VmRSS | `/proc` directory entries |
| macOS | `proc_pidinfo` PROC_PIDTASKINFO (nanoseconds) | `proc_pidinfo` `pti_resident_size` | `proc_listallpids` |
| Windows | `GetProcessTimes` (100 ns intervals) | `K32GetProcessMemoryInfo` WorkingSet | `CreateToolhelp32Snapshot` |

### Process-tree tracking

`stats::collect_descendants` performs a BFS from the root PID using each process's parent PID. CPU time and RSS are summed across the entire subtree, so programs that fork many children are fully accounted for.

## Testing

```bash
cargo test
```

- **Unit tests** (`src/stats.rs`): synthetic `/proc` files covering parsing, malformed input, missing files, and tree traversal. Linux only.
- **Integration tests** (`tests/integration.rs`): real child processes verifying timeout enforcement, memory-limit enforcement, and natural exit detection.
- **CLI tests** (`tests/cli_test.rs`): end-to-end tests against the compiled binary.
