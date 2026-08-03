//! Peak RSS, measured in-process.
//!
//! RSS is read in-process rather than with `/usr/bin/time -v`:
//! GNU `time` is not the shell builtin and Debian slim does not ship it, so the
//! Docker bench stage would need an extra package for a number both runtimes
//! can report about themselves. `getrusage` here, `process.resourceUsage()` on
//! the Node side; both give peak RSS in kilobytes, which makes the methodology
//! uniform rather than merely comparable.
//!
//! Note the metric: `ru_maxrss` is the *high-water mark* for the process, not
//! current usage. That is what we want — it cannot be gamed by measuring after
//! a free — but it does mean the reported figure includes the materialised
//! workload arrays, which is why `bench/methodology.md` states their size and
//! why the honest comparison is the delta over a no-op baseline of the same
//! runtime.

/// Peak resident set size of this process, in kilobytes.
pub fn peak_kb() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();

    // SAFETY: `getrusage` writes a complete `struct rusage` through the
    // pointer on success and touches nothing else. The allocation is a
    // correctly sized, correctly aligned `libc::rusage`, and it is only read
    // after a success return, so it is never read uninitialised.
    let usage = unsafe {
        if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) != 0 {
            return 0;
        }

        usage.assume_init()
    };

    // Linux reports kilobytes. (macOS reports bytes; irrelevant here — the
    // benchmark is Linux-only per  — but worth knowing before
    // anyone ports this.)
    usage.ru_maxrss.max(0) as u64
}
