//! What differs between operating systems, outside the drive backend.

/// Give memory that was freed back to the system.
///
/// glibc keeps freed memory for reuse rather than returning it, so a burst of
/// large, short-lived allocations - decoding a cover, building a label and
/// its mipmaps - would stay on the books for good: about 15 MB, half of
/// Spectra's memory at rest. Elsewhere the allocator returns it by itself.
pub fn release_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim only walks the allocator's own free lists.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Whether the machine is set to save power, which leaves too little CPU for
/// the heavier emulators: Midnight Club ran at half speed in power-saver mode,
/// with half its sound missing, and played smoothly in balanced. The user
/// chooses the mode; Spectra only says so.
///
/// Read from the kernel, as power-profiles-daemon and tuned leave it: the
/// platform profile, or else the CPU's energy preference.
pub fn power_saver() -> bool {
    #[cfg(target_os = "linux")]
    {
        let read = |path: &str| std::fs::read_to_string(path).ok();
        if let Some(profile) = read("/sys/firmware/acpi/platform_profile") {
            return saving(&profile);
        }
        read("/sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference")
            .is_some_and(|preference| preference.trim() == "power")
    }
    #[cfg(not(target_os = "linux"))]
    false
}

/// The platform profiles that save power over speed.
#[cfg(target_os = "linux")]
fn saving(profile: &str) -> bool {
    matches!(profile.trim(), "low-power" | "quiet" | "cool")
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn low_power_and_quiet_save_power_and_balanced_does_not() {
        assert!(super::saving("low-power\n"));
        assert!(super::saving("quiet"));
        assert!(!super::saving("balanced\n"));
        assert!(!super::saving("performance"));
    }
}
