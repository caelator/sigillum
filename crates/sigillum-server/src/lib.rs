//! # Sigillum Server
//!
//! Thin server-facing facade over the local daemon implementation.

pub use sigillum_daemon::{AppState, build_router, run};

#[cfg(test)]
mod tests {
    #[test]
    fn build_router_prepares_base_dir() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let base_dir = temp_dir.path().join("sigillum-server-smoke");

        let (_router, state) =
            crate::build_router(base_dir.clone(), 3200).expect("router should initialize");

        assert!(base_dir.is_dir());
        assert_eq!(&state.base_dir, &base_dir);
    }
}
