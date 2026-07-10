//! Opt-in crash-boundary failpoints for deterministic subprocess tests.
//!
//! The `test-failpoints` feature is never enabled by default. Without it,
//! every call in this module compiles to a no-op and environment variables
//! cannot activate queue hooks in production builds.

pub(super) const AFTER_PREPARED_PERSIST: &str = "queue_after_prepared_persist";
pub(super) const AFTER_SUBMITTED_UNKNOWN_PERSIST: &str = "queue_after_submitted_unknown_persist";

#[cfg(not(feature = "test-failpoints"))]
#[inline]
pub(super) fn hit(_name: &str) {}

#[cfg(feature = "test-failpoints")]
pub(super) fn hit(name: &str) {
    use std::ffi::OsStr;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;

    const ACTIVE_ENV: &str = "SIGILLUM_TEST_FAILPOINT";
    const READY_PATH_ENV: &str = "SIGILLUM_TEST_FAILPOINT_READY_PATH";

    if std::env::var_os(ACTIVE_ENV).as_deref() != Some(OsStr::new(name)) {
        return;
    }

    let ready_path = PathBuf::from(
        std::env::var_os(READY_PATH_ENV)
            .unwrap_or_else(|| panic!("{READY_PATH_ENV} is required when {ACTIVE_ENV}={name}")),
    );
    let parent = ready_path.parent().unwrap_or_else(|| {
        panic!(
            "{READY_PATH_ENV} must have a parent directory: {}",
            ready_path.display()
        )
    });
    fs::create_dir_all(parent).unwrap_or_else(|error| {
        panic!(
            "failed to create failpoint marker directory {}: {error}",
            parent.display()
        )
    });

    let marker_name = ready_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("ready");
    let temporary_path = parent.join(format!(".{marker_name}.{}.tmp", std::process::id()));
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .unwrap_or_else(|error| {
            panic!(
                "failed to create failpoint marker {}: {error}",
                temporary_path.display()
            )
        });
    writeln!(temporary, "{name}").expect("failed to write failpoint marker");
    temporary
        .sync_all()
        .expect("failed to sync failpoint marker contents");
    fs::rename(&temporary_path, &ready_path).unwrap_or_else(|error| {
        panic!(
            "failed to publish failpoint marker {}: {error}",
            ready_path.display()
        )
    });

    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .unwrap_or_else(|error| {
            panic!(
                "failed to sync failpoint marker directory {}: {error}",
                parent.display()
            )
        });

    // Only SIGKILL from the parent proof should let this process cross the
    // selected boundary. Looping protects against spurious unparks.
    loop {
        std::thread::park();
    }
}
