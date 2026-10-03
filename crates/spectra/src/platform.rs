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
