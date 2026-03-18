//! Write-blocker pre-flight verification.
//!
//! A hardware write-blocker prevents any write access to the source drive.
//! Before imaging begins we attempt to open the device path in write mode:
//!
//! * Open **fails** → write-blocking is active (or OS-level denial) → **safe**.
//! * Open **succeeds** → write-blocking is NOT active → **warn the operator**.
//!
//! The function is intentionally pure and side-effect free: the returned file
//! handle is dropped immediately without writing any data.

/// Returns `true` if write-blocking appears active (device could not be opened
/// for writing), or `false` if the device is writable (no hardware write-blocker
/// detected).
///
/// An empty `device_path` returns `true` (safe default — treat unknown as blocked).
pub fn check(device_path: &str) -> bool {
    if device_path.is_empty() {
        return true;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(device_path)
        .map(|_| false) // opened for writing → NOT blocked (warning)
        .unwrap_or(true) // error opening → blocked (or OS denied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_returns_true() {
        assert!(check(""));
    }

    #[test]
    fn nonexistent_path_returns_true() {
        // A path that does not exist → open fails → treated as blocked (safe).
        assert!(check("/nonexistent/ferrite_wb_test_device_XYZ"));
    }

    #[test]
    fn writable_temp_file_returns_false() {
        let path = std::env::temp_dir().join("ferrite_wb_writable_test.bin");
        std::fs::write(&path, b"ferrite").unwrap();
        let result = check(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(!result, "writable file should return false (not blocked)");
    }

    #[test]
    fn readonly_file_returns_true() {
        let path = std::env::temp_dir().join("ferrite_wb_readonly_test.bin");
        std::fs::write(&path, b"ferrite").unwrap();

        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let result = check(path.to_str().unwrap());

        // Restore write permission before cleanup.
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut p = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            std::fs::set_permissions(&path, p).ok();
        }
        std::fs::remove_file(&path).ok();

        assert!(result, "read-only file should return true (blocked)");
    }

    #[test]
    fn repeated_calls_are_idempotent() {
        // Calling check twice on the same path gives the same result.
        let path = std::env::temp_dir().join("ferrite_wb_idempotent_test.bin");
        std::fs::write(&path, b"ferrite").unwrap();
        let r1 = check(path.to_str().unwrap());
        let r2 = check(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert_eq!(r1, r2);
    }
}
