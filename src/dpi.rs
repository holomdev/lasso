//! DPI awareness bootstrap.
//!
//! This must run before any window or DC is created.
//!
//! `PER_MONITOR_AWARE_V2` is preferred over the legacy
//! `PROCESS_PER_MONITOR_DPI_AWARE` mode because it fixes that mode's
//! coordinate-space behaviour while still keeping every monitor at its own
//! physical pixel resolution. That is what allows a selection rectangle to span
//! monitors with different scale factors without drifting.
//!
//! If the process is already DPI aware (for example through an embedded
//! application manifest), both calls fail and `Unchanged` is returned.

use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwareness, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, PROCESS_PER_MONITOR_DPI_AWARE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpiAwareness {
    /// `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` (Windows 10 1703+).
    PerMonitorV2,
    /// `PROCESS_PER_MONITOR_DPI_AWARE` (Windows 8.1+).
    PerMonitor,
    /// Already fixed by another mechanism, or the change was refused.
    Unchanged,
}

impl DpiAwareness {
    pub fn as_str(self) -> &'static str {
        match self {
            DpiAwareness::PerMonitorV2 => "per-monitor-v2",
            DpiAwareness::PerMonitor => "per-monitor",
            DpiAwareness::Unchanged => "unchanged",
        }
    }
}

pub fn enable_per_monitor() -> DpiAwareness {
    unsafe {
        if SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_ok() {
            return DpiAwareness::PerMonitorV2;
        }
        if SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE).is_ok() {
            return DpiAwareness::PerMonitor;
        }
    }
    DpiAwareness::Unchanged
}
