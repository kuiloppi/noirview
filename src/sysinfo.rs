//! Small OS helpers: keep background decoders below the UI and Windows in the
//! scheduler, and read this process's memory use for the safety guard.

/// Run the calling (worker) thread below normal priority so the UI thread and
/// other programs stay responsive while thumbnails decode. No-op elsewhere.
pub fn lower_current_thread_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Put the calling thread in Windows background mode: lowest CPU priority
/// and very low disk and memory priority, so it only uses what nothing else
/// wants. Elsewhere it lowers the CPU priority only.
pub fn enter_background_mode() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        if SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) == 0 {
            lower_current_thread_priority();
        }
    }
    #[cfg(not(windows))]
    lower_current_thread_priority();
}

/// Private memory committed by this process, in bytes.
pub fn process_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetCurrentProcess,
        };
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        (ok != 0).then_some(counters.PagefileUsage as u64)
    }
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(resident * 4096)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_is_reported_and_priority_call_is_harmless() {
        super::lower_current_thread_priority();
        #[cfg(any(windows, target_os = "linux"))]
        assert!(super::process_memory().unwrap() > 1024 * 1024);
    }
}
