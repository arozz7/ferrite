//! SHA-256 image integrity hashing.
//!
//! Reads the completed output image file sequentially and produces a lowercase
//! hex-encoded digest.  A companion `.sha256` sidecar file is written alongside
//! the image so the hash survives across sessions without re-computing.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Compute the SHA-256 digest of the file at `path`.
///
/// Reads the file in 64 KiB chunks.  Returns `None` on any I/O error.
pub fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// Return the sidecar path for `image_path`: `<image_path>.sha256`.
pub fn sidecar_path(image_path: &Path) -> PathBuf {
    let mut p = image_path.as_os_str().to_owned();
    p.push(".sha256");
    PathBuf::from(p)
}

/// Compute the SHA-256 of `image_path`, write the result to the companion
/// `.sha256` sidecar file, and return the hex string.
///
/// The sidecar contains a single line:
/// ```text
/// <hex>  <filename>\n
/// ```
/// which is compatible with `sha256sum --check`.
///
/// Returns `None` when hashing fails (I/O error on the image file).  Sidecar
/// write failures are logged as warnings but do not cause `None` to be returned.
pub fn hash_and_save(image_path: &Path) -> Option<String> {
    let hex = hash_file(image_path)?;
    let sidecar = sidecar_path(image_path);
    let filename = image_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| image_path.to_string_lossy().into_owned());
    let line = format!("{}  {}\n", hex, filename);
    match std::fs::write(&sidecar, &line) {
        Ok(()) => {
            info!(path = %sidecar.display(), "imaging: SHA-256 sidecar written");
        }
        Err(e) => {
            warn!(path = %sidecar.display(), error = %e, "imaging: could not write SHA-256 sidecar");
        }
    }
    Some(hex)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_file_known_value() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let hex = hash_file(tmp.path()).unwrap();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hash_file_nonempty() {
        use std::io::Write as _;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"ferrite").unwrap();
        tmp.flush().unwrap();
        let hex = hash_file(tmp.path()).unwrap();
        // Verify it is a 64-char hex string.
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_and_save_writes_sidecar() {
        use std::io::Write as _;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        {
            let mut f = tmp.reopen().unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let hex = hash_and_save(tmp.path()).unwrap();
        assert_eq!(hex.len(), 64);

        let sidecar = sidecar_path(tmp.path());
        let content = std::fs::read_to_string(&sidecar).unwrap();
        assert!(content.starts_with(&hex));
        let _ = std::fs::remove_file(&sidecar);
    }

    #[test]
    fn sidecar_path_appends_extension() {
        let p = Path::new("/tmp/disk.img");
        assert_eq!(sidecar_path(p), PathBuf::from("/tmp/disk.img.sha256"));
    }

    #[test]
    fn hash_file_nonexistent_returns_none() {
        assert!(hash_file(Path::new("/nonexistent/path/file.img")).is_none());
    }
}
