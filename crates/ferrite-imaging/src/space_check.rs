//! Pre-flight destination disk-space check.
//!
//! Queries the free space on the filesystem that hosts the destination path and
//! compares it against the required image size.  Used by the TUI to warn the
//! operator before imaging begins rather than letting the run fail mid-way.

use std::path::{Path, PathBuf};

/// Free-space information relative to a required image size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceInfo {
    /// Bytes available to the current user at the destination filesystem.
    pub available: u64,
    /// Bytes required for the image (device size or LBA-range size).
    pub required: u64,
}

impl SpaceInfo {
    /// Ratio of `available / required`.  Returns `1.0` when `required == 0`.
    pub fn ratio(&self) -> f64 {
        if self.required == 0 {
            1.0
        } else {
            self.available as f64 / self.required as f64
        }
    }

    /// `true` when `available >= required`.
    pub fn sufficient(&self) -> bool {
        self.available >= self.required
    }
}

/// Query free space for the filesystem hosting `dest_path` and compare against
/// `required` bytes.
///
/// Walks up `dest_path` to find the nearest existing ancestor (useful when the
/// user has typed a path whose parent directories do not yet exist).
///
/// Returns `None` when `dest_path` is empty, no existing ancestor could be
/// found, or the OS query fails.
pub fn check(dest_path: &Path, required: u64) -> Option<SpaceInfo> {
    if dest_path.as_os_str().is_empty() {
        return None;
    }
    let query = existing_ancestor(dest_path)?;
    let available = fs2::available_space(&query).ok()?;
    Some(SpaceInfo {
        available,
        required,
    })
}

/// Walk up `path` to find the first ancestor that exists on disk.
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut p = path.to_path_buf();
    loop {
        if p.exists() {
            return Some(p);
        }
        let parent = p.parent()?.to_path_buf();
        if parent == p {
            return None;
        }
        p = parent;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_required_zero_returns_one() {
        let s = SpaceInfo {
            available: 100,
            required: 0,
        };
        assert_eq!(s.ratio(), 1.0);
    }

    #[test]
    fn ratio_half() {
        let s = SpaceInfo {
            available: 50,
            required: 100,
        };
        assert!((s.ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn sufficient_when_equal() {
        let s = SpaceInfo {
            available: 100,
            required: 100,
        };
        assert!(s.sufficient());
    }

    #[test]
    fn insufficient_when_less() {
        let s = SpaceInfo {
            available: 99,
            required: 100,
        };
        assert!(!s.sufficient());
    }

    #[test]
    fn check_empty_path_returns_none() {
        assert!(check(Path::new(""), 1024).is_none());
    }

    #[test]
    fn check_temp_dir_returns_some() {
        let tmp = std::env::temp_dir();
        let result = check(&tmp, 1024);
        assert!(result.is_some(), "temp dir should return Some");
    }

    #[test]
    fn check_nonexistent_child_resolves_to_parent_volume() {
        let path = std::env::temp_dir().join("ferrite_space_check_nonexistent_XYZ.img");
        let result = check(&path, 1024);
        assert!(
            result.is_some(),
            "non-existent child of temp dir should resolve to parent volume"
        );
    }
}
