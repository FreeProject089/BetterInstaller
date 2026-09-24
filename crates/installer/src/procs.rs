//! Closing the app's own running processes before its files are replaced or removed.
//!
//! By FULL IMAGE PATH (card C-6). It used to be `taskkill /F /IM <name>`, and `/IM`
//! matches an image name across the whole system: updating `D:\Apps\BMM` also closed a
//! second copy of `better-mods-manager.exe` running from `E:\Portable\BMM`, or any other
//! program that happened to share a file name with one of the package's executables.
//!
//! Now each process is looked up with Toolhelp32, its image path is read with
//! `QueryFullProcessImageNameW`, and it is closed only when that path is one of the files
//! being replaced. Both sides are canonicalised before the comparison, so a short (8.3)
//! name, a different case or a `..` in either one does not decide the answer.

use std::path::{Path, PathBuf};

/// Terminate every process of this user whose executable is one of `targets` (never this
/// process). Waits up to 5 s for each to exit, so its files are released. Returns how many
/// were closed. Processes that cannot be opened (another user's, elevated) are skipped.
#[cfg(windows)]
pub fn close_by_path(targets: &[PathBuf]) -> usize {
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessEntry32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }
    type Handle = isize;
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(h: Handle, flags: u32, buf: *mut u16, size: *mut u32) -> i32;
        fn TerminateProcess(h: Handle, code: u32) -> i32;
        fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
        fn CloseHandle(h: Handle) -> i32;
    }
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: Handle = -1;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SYNCHRONIZE: u32 = 0x0010_0000;

    // What to look for: the canonical path, and its file name as a cheap first filter so
    // only processes with a matching name are opened at all.
    let wanted: Vec<(PathBuf, String)> = targets
        .iter()
        .filter_map(|t| {
            let canon = std::fs::canonicalize(t).ok()?;
            let name = canon.file_name()?.to_string_lossy().to_lowercase();
            Some((canon, name))
        })
        .collect();
    if wanted.is_empty() {
        return 0;
    }
    let me = std::process::id();
    let mut closed = 0;

    // SAFETY: documented kernel32 calls. The entry struct is the Win32 layout with dwSize
    // set before the first call; every handle opened here is closed on every path.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap == 0 {
            return 0;
        }
        let mut e: ProcessEntry32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let pid = e.th32ProcessID;
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            let name = std::ffi::OsString::from_wide(&e.szExeFile[..len])
                .to_string_lossy()
                .to_lowercase();
            if pid != me && pid != 0 && wanted.iter().any(|(_, n)| *n == name) {
                let h = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE,
                    0,
                    pid,
                );
                if h != 0 {
                    // The path is read through the SAME handle that would terminate it, so a
                    // pid reused between the snapshot and here cannot be mistaken for it.
                    let mut buf = [0u16; 32768];
                    let mut size = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) != 0 {
                        let image =
                            PathBuf::from(std::ffi::OsString::from_wide(&buf[..size as usize]));
                        let is_target = std::fs::canonicalize(&image)
                            .map(|c| wanted.iter().any(|(w, _)| same_path(w, &c)))
                            .unwrap_or(false);
                        if is_target && TerminateProcess(h, 1) != 0 {
                            WaitForSingleObject(h, 5000);
                            closed += 1;
                        }
                    }
                    CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    closed
}

#[cfg(not(windows))]
pub fn close_by_path(_targets: &[PathBuf]) -> usize {
    0
}

/// Windows paths compare without regard to case.
#[cfg_attr(not(windows), allow(dead_code))]
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// A long-running harmless process started from `dir` under `name`: a copy of the
    /// system's ping.exe pinging the loopback address for a minute.
    fn run_copy(dir: &Path, name: &str) -> Child {
        std::fs::create_dir_all(dir).unwrap();
        let sys = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let exe = dir.join(name);
        std::fs::copy(Path::new(&sys).join("System32").join("PING.EXE"), &exe).unwrap();
        Command::new(&exe)
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn exited_within(c: &mut Child, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if c.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Card C-6. Two copies of the same program, same file name, two folders. Updating the
    /// first must close the first and only the first.
    #[test]
    fn only_the_copy_in_the_folder_being_updated_is_closed() {
        let base = std::env::temp_dir().join(format!("bi-c6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // A unique name, so nothing else on the machine can share it.
        let name = format!("bi-c6-probe-{}.exe", std::process::id());
        let mut ours = run_copy(&base.join("install"), &name);
        let mut other = run_copy(&base.join("elsewhere"), &name);
        std::thread::sleep(Duration::from_millis(300));
        assert!(ours.try_wait().unwrap().is_none(), "probe did not start");
        assert!(other.try_wait().unwrap().is_none(), "probe did not start");

        let n = close_by_path(&[base.join("install").join(&name)]);

        let ours_gone = exited_within(&mut ours, Duration::from_secs(5));
        let other_alive = other.try_wait().unwrap().is_none();
        let _ = ours.kill();
        let _ = other.kill();
        let _ = ours.wait();
        let _ = other.wait();
        let _ = std::fs::remove_dir_all(&base);
        assert!(
            ours_gone,
            "the process in the install folder is still running"
        );
        assert!(
            other_alive,
            "a copy of the same program in ANOTHER folder was closed"
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn a_path_that_does_not_exist_closes_nothing() {
        assert_eq!(
            close_by_path(&[PathBuf::from("Z:\\no\\such\\dir\\app.exe")]),
            0
        );
    }
}
