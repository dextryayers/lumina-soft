//! Process memory measurement.
//!
//! The performance budget is expressed as resident set size, not heap size:
//! mapped GPU buffers and shared libraries count too. Windows reports
//! `WorkingSetSize` through `GetProcessMemoryInfo`; Linux reports `VmRSS` in
//! `/proc/self/status`. Unknown platforms return `None` rather than zero, so a
//! missing measurement can never be mistaken for a lean process.
//!
//! Measurement is on demand, not per frame. Reading `/proc` or calling PSAPI is
//! far too expensive for a debug overlay that redraws while it is open.

/// Resident and peak resident memory for the current process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemory {
    resident_bytes: u64,
    peak_bytes: Option<u64>,
}

impl ProcessMemory {
    /// Construct a reading. Used by the platform implementations and tests.
    #[must_use]
    pub const fn new(resident_bytes: u64, peak_bytes: Option<u64>) -> Self {
        Self {
            resident_bytes,
            peak_bytes,
        }
    }

    /// Current resident set size in bytes.
    #[must_use]
    pub const fn resident_bytes(self) -> u64 {
        self.resident_bytes
    }

    /// Peak resident set size in bytes, when the platform reports it.
    #[must_use]
    pub const fn peak_bytes(self) -> Option<u64> {
        self.peak_bytes
    }

    /// Current resident set size in mebibytes.
    #[must_use]
    pub fn resident_mib(self) -> f32 {
        self.resident_bytes as f32 / (1024.0 * 1024.0)
    }

    /// Peak resident set size in mebibytes, when known.
    #[must_use]
    pub fn peak_mib(self) -> Option<f32> {
        self.peak_bytes
            .map(|bytes| bytes as f32 / (1024.0 * 1024.0))
    }

    /// A short diagnostic line such as `rss 18.4 MiB (peak 24.1 MiB)`.
    #[must_use]
    pub fn diagnostic_line(self) -> String {
        match self.peak_mib() {
            Some(peak) => format!("rss {:.1} MiB (peak {:.1} MiB)", self.resident_mib(), peak),
            None => format!("rss {:.1} MiB", self.resident_mib()),
        }
    }
}

/// Read the current process's resident memory, if the platform exposes it.
#[must_use]
pub fn process_memory() -> Option<ProcessMemory> {
    platform_memory()
}

#[cfg(target_os = "linux")]
fn platform_memory() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_linux_status(&status)
}

#[cfg(target_os = "windows")]
fn platform_memory() -> Option<ProcessMemory> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb).ok()?;
    }
    Some(ProcessMemory::new(
        counters.WorkingSetSize as u64,
        Some(counters.PeakWorkingSetSize as u64),
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_memory() -> Option<ProcessMemory> {
    None
}

/// Parse the `VmRSS` and `VmHWM` lines from `/proc/self/status`.
///
/// Kept platform-independent so Linux parsing is testable on every host.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_linux_status(status: &str) -> Option<ProcessMemory> {
    fn kib_line(status: &str, key: &str) -> Option<u64> {
        let line = status.lines().find(|line| line.starts_with(key))?;
        let value = line.strip_prefix(key)?.split_whitespace().next()?;
        value.parse::<u64>().ok().map(|kib| kib * 1024)
    }

    let resident = kib_line(status, "VmRSS:")?;
    let peak = kib_line(status, "VmHWM:");
    Some(ProcessMemory::new(resident, peak))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_status_parses_rss_and_peak() {
        let status =
            "Name:\tlumina\nVmPeak:\t  204800 kB\nVmHWM:\t   24576 kB\nVmRSS:\t   18432 kB\n";
        let memory = parse_linux_status(status).expect("the status has VmRSS");
        assert_eq!(memory.resident_bytes(), 18_432 * 1024);
        assert_eq!(memory.peak_bytes(), Some(24_576 * 1024));
        assert!((memory.resident_mib() - 18.0).abs() < 0.1);
        assert!(memory.diagnostic_line().contains("rss 18.0 MiB"));
    }

    #[test]
    fn missing_peak_is_not_an_error() {
        let memory = parse_linux_status("VmRSS:\t  1024 kB\n").expect("VmRSS is enough");
        assert_eq!(memory.resident_bytes(), 1_048_576);
        assert_eq!(memory.peak_bytes(), None);
        assert_eq!(memory.diagnostic_line(), "rss 1.0 MiB");
    }

    #[test]
    fn missing_rss_returns_none() {
        assert_eq!(parse_linux_status("Name:\tlumina\n"), None);
        assert_eq!(parse_linux_status("VmRSS:\tnot-a-number kB\n"), None);
    }

    #[test]
    fn invalid_peak_is_ignored_but_rss_is_kept() {
        let memory = parse_linux_status("VmHWM:\tbad kB\nVmRSS:\t2048 kB\n").expect("VmRSS");
        assert_eq!(memory.peak_bytes(), None);
    }

    #[test]
    fn a_live_reading_is_plausible_when_available() {
        if let Some(memory) = process_memory() {
            assert!(memory.resident_bytes() > 0);
            assert!(memory.resident_mib() > 0.1);
            if let Some(peak) = memory.peak_bytes() {
                assert!(peak >= memory.resident_bytes());
            }
        }
    }
}
