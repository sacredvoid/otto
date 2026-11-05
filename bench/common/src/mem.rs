/// Peak resident memory of this process in MB (0 on unsupported targets).
#[cfg(windows)]
pub fn peak_memory_mb() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: c is a valid zeroed out-struct with cb set to its size.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return 0;
        }
        (c.PeakWorkingSetSize as u64) / (1024 * 1024)
    }
}

#[cfg(target_os = "macos")]
pub fn peak_memory_mb() -> u64 {
    // SAFETY: u is a valid out-struct; ru_maxrss is in bytes on macOS.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut u) != 0 {
            return 0;
        }
        (u.ru_maxrss as u64) / (1024 * 1024)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn peak_memory_mb() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn peak_memory_is_nonzero_on_supported_targets() {
        if cfg!(any(windows, target_os = "macos")) {
            assert!(super::peak_memory_mb() > 0);
        }
    }
}
