use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuTimes {
    pub user: Duration,
    pub sys: Duration,
}

// ─────────────────────────────── Linux ────────────────────────────────────────

#[cfg(target_os = "linux")]
fn proc_dir() -> String {
    std::env::var("FMS_PROC_DIR").unwrap_or_else(|_| "/proc".to_string())
}

#[cfg(target_os = "linux")]
pub fn read_cpu_times(pid: u32) -> Option<CpuTimes> {
    use std::fs;
    let content = fs::read_to_string(format!("{}/{}/stat", proc_dir(), pid)).ok()?;
    let after_comm = content.rfind(')')?.checked_add(1)?;
    let rest = &content[after_comm..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let ticks = ticks_per_sec();
    Some(CpuTimes {
        user: Duration::from_secs_f64(utime as f64 / ticks),
        sys: Duration::from_secs_f64(stime as f64 / ticks),
    })
}

#[cfg(target_os = "linux")]
pub fn read_ppid(pid: u32) -> Option<u32> {
    use std::fs;
    let content = fs::read_to_string(format!("{}/{}/stat", proc_dir(), pid)).ok()?;
    let after_comm = content.rfind(')')?.checked_add(1)?;
    let rest = &content[after_comm..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
pub fn read_mem_kb(pid: u32) -> Option<u64> {
    use std::fs;
    let content = fs::read_to_string(format!("{}/{}/status", proc_dir(), pid)).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub fn all_pids() -> Vec<u32> {
    use std::fs;
    let Ok(entries) = fs::read_dir(proc_dir()) else { return vec![]; };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}

#[cfg(target_os = "linux")]
fn ticks_per_sec() -> f64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 { 100.0 } else { ticks as f64 }
}

// ─────────────────────────────── macOS ────────────────────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "proc")]
extern "C" {
    fn proc_listallpids(buffer: *mut libc::c_void, buffersize: libc::c_int) -> libc::c_int;
    fn proc_pidinfo(
        pid: libc::c_int,
        flavor: libc::c_int,
        arg: u64,
        buffer: *mut libc::c_void,
        buffersize: libc::c_int,
    ) -> libc::c_int;
}

// proc_taskinfo (flavor 4): CPU times in nanoseconds, RSS in bytes.
#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcTaskInfo {
    pti_virtual_size:      u64,
    pti_resident_size:     u64,
    pti_total_user:        u64,
    pti_total_system:      u64,
    pti_threads_user:      u64,
    pti_threads_system:    u64,
    pti_policy:            i32,
    pti_faults:            i32,
    pti_pageins:           i32,
    pti_cow_faults:        i32,
    pti_messages_sent:     i32,
    pti_messages_received: i32,
    pti_syscalls_mach:     i32,
    pti_syscalls_unix:     i32,
    pti_csw:               i32,
    pti_threadnum:         i32,
    pti_numrunning:        i32,
    pti_priority:          i32,
}

// proc_bsdshortinfo (flavor 13): parent PID and basic identity.
#[cfg(target_os = "macos")]
#[repr(C)]
struct ProcBsdShortInfo {
    pbsi_pid:      u32,
    pbsi_ppid:     u32,
    pbsi_pgid:     u32,
    pbsi_status:   u32,
    pbsi_comm:     [libc::c_char; 16],
    pbsi_flags:    u32,
    pbsi_uid:      u32,
    pbsi_gid:      u32,
    pbsi_ruid:     u32,
    pbsi_rgid:     u32,
    pbsi_svuid:    u32,
    pbsi_svgid:    u32,
    pbsi_reserved: u32,
}

#[cfg(target_os = "macos")]
fn macos_task_info(pid: u32) -> Option<ProcTaskInfo> {
    let mut info: ProcTaskInfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid as libc::c_int,
            4, // PROC_PIDTASKINFO
            0,
            &mut info as *mut _ as *mut libc::c_void,
            std::mem::size_of::<ProcTaskInfo>() as libc::c_int,
        )
    };
    if ret as usize != std::mem::size_of::<ProcTaskInfo>() { return None; }
    Some(info)
}

#[cfg(target_os = "macos")]
pub fn read_cpu_times(pid: u32) -> Option<CpuTimes> {
    let info = macos_task_info(pid)?;
    Some(CpuTimes {
        user: Duration::from_nanos(info.pti_total_user),
        sys:  Duration::from_nanos(info.pti_total_system),
    })
}

#[cfg(target_os = "macos")]
pub fn read_ppid(pid: u32) -> Option<u32> {
    let mut info: ProcBsdShortInfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid as libc::c_int,
            13, // PROC_PIDT_SHORTBSDINFO
            0,
            &mut info as *mut _ as *mut libc::c_void,
            std::mem::size_of::<ProcBsdShortInfo>() as libc::c_int,
        )
    };
    if ret as usize != std::mem::size_of::<ProcBsdShortInfo>() { return None; }
    Some(info.pbsi_ppid)
}

#[cfg(target_os = "macos")]
pub fn read_mem_kb(pid: u32) -> Option<u64> {
    let info = macos_task_info(pid)?;
    Some(info.pti_resident_size / 1024)
}

#[cfg(target_os = "macos")]
pub fn all_pids() -> Vec<u32> {
    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 { return vec![]; }
    let mut buf = vec![0i32; count as usize + 32];
    let actual = unsafe {
        proc_listallpids(
            buf.as_mut_ptr() as *mut libc::c_void,
            (buf.len() * std::mem::size_of::<i32>()) as libc::c_int,
        )
    };
    if actual <= 0 { return vec![]; }
    let actual = (actual as usize).min(buf.len());
    buf[..actual].iter().map(|&p| p as u32).collect()
}

// ────────────────────────────── Windows ───────────────────────────────────────

#[cfg(windows)]
pub fn read_cpu_times(pid: u32) -> Option<CpuTimes> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if handle == 0 { return None; }
        let mut c = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut e = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut k = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut u = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let ok = GetProcessTimes(handle, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(handle);
        if ok == 0 { return None; }
        // FILETIME units are 100 ns intervals
        let user_ns   = (((u.dwHighDateTime as u64) << 32) | u.dwLowDateTime as u64) * 100;
        let kernel_ns = (((k.dwHighDateTime as u64) << 32) | k.dwLowDateTime as u64) * 100;
        Some(CpuTimes {
            user: Duration::from_nanos(user_ns),
            sys:  Duration::from_nanos(kernel_ns),
        })
    }
}

#[cfg(windows)]
pub fn read_ppid(pid: u32) -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE { return None; }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut result = None;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID == pid {
                    result = Some(entry.th32ParentProcessID);
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 { break; }
            }
        }
        CloseHandle(snap);
        result
    }
}

#[cfg(windows)]
pub fn read_mem_kb(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION};
    unsafe {
        // PROCESS_VM_READ (0x10) is also needed by some versions of GetProcessMemoryInfo
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | 0x0010, 0, pid);
        if handle == 0 { return None; }
        let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = K32GetProcessMemoryInfo(handle, &mut mem, mem.cb);
        CloseHandle(handle);
        if ok == 0 { return None; }
        Some(mem.WorkingSetSize as u64 / 1024)
    }
}

#[cfg(windows)]
pub fn all_pids() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let mut pids = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE { return pids; }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                pids.push(entry.th32ProcessID);
                if Process32NextW(snap, &mut entry) == 0 { break; }
            }
        }
        CloseHandle(snap);
    }
    pids
}

// ──────────────────────────── platform-agnostic ───────────────────────────────

/// Recursively collects all descendant pids of root_pid (not including root itself).
pub fn collect_descendants(root_pid: u32) -> Vec<u32> {
    let pids = all_pids();
    let mut result = Vec::new();
    let mut queue = vec![root_pid];
    while let Some(parent) = queue.pop() {
        for &pid in &pids {
            if read_ppid(pid) == Some(parent) && pid != root_pid {
                result.push(pid);
                queue.push(pid);
            }
        }
    }
    result
}

/// Sum CPU times across root_pid and all its descendants.
pub fn tree_cpu_times(root_pid: u32) -> CpuTimes {
    let pids = collect_descendants(root_pid);
    let mut total_user = Duration::ZERO;
    let mut total_sys = Duration::ZERO;
    if let Some(t) = read_cpu_times(root_pid) {
        total_user += t.user;
        total_sys += t.sys;
    }
    for pid in pids {
        if let Some(t) = read_cpu_times(pid) {
            total_user += t.user;
            total_sys += t.sys;
        }
    }
    CpuTimes { user: total_user, sys: total_sys }
}

/// Sum RSS across root_pid and all descendants.
pub fn tree_mem_kb(root_pid: u32) -> u64 {
    let mut total = 0u64;
    let mut pids = collect_descendants(root_pid);
    pids.push(root_pid);
    for pid in pids {
        total += read_mem_kb(pid).unwrap_or(0);
    }
    total
}

// ─────────────── tests (Linux only — rely on /proc mocking) ───────────────────

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct MockProc {
        dir: PathBuf,
    }

    impl MockProc {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("fms_test_proc_{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn create_process(&self, pid: u32, ppid: u32, utime: u64, stime: u64, rss_kb: u64) {
            let pid_dir = self.dir.join(pid.to_string());
            fs::create_dir_all(&pid_dir).unwrap();

            let mut after_comm = vec!["X".to_string(); 15];
            after_comm[0] = "S".to_string();
            after_comm[1] = ppid.to_string();
            after_comm[11] = utime.to_string();
            after_comm[12] = stime.to_string();

            let stat_str = format!("{} (test_proc) {}", pid, after_comm.join(" "));
            fs::write(pid_dir.join("stat"), stat_str).unwrap();

            let status_str = format!("Name:\ttest_proc\nState:\tS (sleeping)\nVmRSS:\t{} kB\n", rss_kb);
            fs::write(pid_dir.join("status"), status_str).unwrap();
        }
    }

    impl Drop for MockProc {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn should_parse_cpu_times_correctly() {
        let guard = ENV_MUTEX.lock().unwrap();
        let mock = MockProc::new();
        unsafe { std::env::set_var("FMS_PROC_DIR", &mock.dir); }

        let pid = 123;
        let stat_content = "123 (bash) S 1 123 123 0 -1 4210944 11624 233511 0 0 1000 500 0 0 20 0 1 0 541094 34091008 1747 18446744073709551615 94056763592704 94056764516301 140722216521504 0 0 0 65536 3686404 1266777851 0 0 0 17 3 0 0 0 0 0 94056764660304 94056764665424 94056784384000 140722216526146 140722216526168 140722216526168 140722216529367 0";
        fs::create_dir_all(mock.dir.join(pid.to_string())).unwrap();
        fs::write(mock.dir.join(pid.to_string()).join("stat"), stat_content).unwrap();

        let times = read_cpu_times(pid).unwrap();
        let ticks = ticks_per_sec();
        assert_eq!(times.user, Duration::from_secs_f64(1000.0 / ticks));
        assert_eq!(times.sys,  Duration::from_secs_f64(500.0  / ticks));

        drop(guard);
    }

    #[test]
    fn should_parse_memory_kb() {
        let guard = ENV_MUTEX.lock().unwrap();
        let mock = MockProc::new();
        unsafe { std::env::set_var("FMS_PROC_DIR", &mock.dir); }

        mock.create_process(200, 1, 100, 100, 4096);
        assert_eq!(read_mem_kb(200).unwrap(), 4096);

        drop(guard);
    }

    #[test]
    fn should_build_process_tree() {
        let guard = ENV_MUTEX.lock().unwrap();
        let mock = MockProc::new();
        unsafe { std::env::set_var("FMS_PROC_DIR", &mock.dir); }

        // hierarchy: 1 -> 10 -> 20 -> 30
        mock.create_process(10, 1, 0, 0, 0);
        mock.create_process(20, 10, 0, 0, 0);
        mock.create_process(30, 20, 0, 0, 0);

        let mut desc = collect_descendants(10);
        desc.sort();
        assert_eq!(desc, vec![20, 30]);

        drop(guard);
    }

    #[test]
    fn should_handle_bad_files() {
        let guard = ENV_MUTEX.lock().unwrap();
        let mock = MockProc::new();
        unsafe { std::env::set_var("FMS_PROC_DIR", &mock.dir); }

        assert_eq!(read_cpu_times(999), None);
        assert_eq!(read_ppid(999), None);
        assert_eq!(read_mem_kb(999), None);

        let pid_dir = mock.dir.join("888");
        fs::create_dir_all(&pid_dir).unwrap();
        fs::write(pid_dir.join("stat"), "888 (test)").unwrap();
        assert_eq!(read_cpu_times(888), None);
        assert_eq!(read_ppid(888), None);

        fs::write(pid_dir.join("status"), "Name:\tbroken\nVmRSS: abc kB").unwrap();
        assert_eq!(read_mem_kb(888), None);

        fs::create_dir_all(mock.dir.join("not_a_pid")).unwrap();
        fs::write(mock.dir.join("also_not_pid"), "").unwrap();
        let pids = all_pids();
        assert!(pids.contains(&888));

        drop(guard);
    }

    #[test]
    fn should_handle_missing_proc_dir() {
        let guard = ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("FMS_PROC_DIR", "/totally_fake_missing_folder_xyzzzz8"); }
        assert_eq!(all_pids(), vec![]);
        drop(guard);
    }

    #[test]
    fn should_handle_unparseable_data() {
        let guard = ENV_MUTEX.lock().unwrap();
        let mock = MockProc::new();
        unsafe { std::env::set_var("FMS_PROC_DIR", &mock.dir); }

        let pid_dir = mock.dir.join("777");
        fs::create_dir_all(&pid_dir).unwrap();

        let bad_stat = "777 (test) R 1 1 1 1 1 1 1 1 1 1 bad_utime bad_stime 1 1";
        fs::write(pid_dir.join("stat"), bad_stat).unwrap();
        assert_eq!(read_cpu_times(777), None);

        let bad_stat2 = "777 (test) R 1 1 1 1 1 1 1 1 1 1 100 bad_stime 1 1";
        fs::write(pid_dir.join("stat"), bad_stat2).unwrap();
        assert_eq!(read_cpu_times(777), None);

        let bad_stat_ppid = "777 (test) R unparseable_ppid";
        fs::write(pid_dir.join("stat"), bad_stat_ppid).unwrap();
        assert_eq!(read_ppid(777), None);

        fs::create_dir_all(pid_dir.join("status")).unwrap();
        assert_eq!(read_mem_kb(777), None);

        fs::create_dir_all(mock.dir.join("not_a_number_dir")).unwrap();
        let pids = all_pids();
        assert!(!pids.contains(&9999999));

        drop(guard);
    }

    #[test]
    fn should_handle_read_dir_error() {
        let guard = ENV_MUTEX.lock().unwrap();
        let temp_file = std::env::temp_dir().join("fms_proc_mock_file");
        fs::write(&temp_file, "mock").unwrap();
        unsafe { std::env::set_var("FMS_PROC_DIR", &temp_file); }
        assert_eq!(all_pids(), vec![]);
        fs::remove_file(&temp_file).unwrap();
        drop(guard);
    }
}
