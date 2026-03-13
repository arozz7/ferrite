//! `ferrite-carver` — signature-based file carving engine.
//!
//! # Overview
//!
//! ```ignore
//! use std::sync::Arc;
//! use ferrite_carver::{Carver, CarvingConfig};
//!
//! // Load signatures from the bundled TOML
//! let cfg = CarvingConfig::from_toml_str(include_str!("../config/signatures.toml"))?;
//!
//! // Scan for all known file types
//! let carver = Carver::new(Arc::clone(&device), cfg);
//! let hits = carver.scan()?;
//!
//! // Extract each hit to a Vec<u8>
//! for hit in &hits {
//!     let mut out = Vec::new();
//!     let bytes = carver.extract(hit, &mut out)?;
//!     println!("Found {} ({} bytes) at offset {}", hit.signature.name, bytes, hit.byte_offset);
//! }
//! ```

mod error;
mod scanner;
mod signature;

pub use error::{CarveError, Result};
pub use scanner::{CarveHit, Carver, ScanProgress};
pub use signature::{parse_hex, CarvingConfig, Signature};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrite_blockdev::{BlockDevice, MockBlockDevice};

    use super::*;

    /// Smoke-test: load the real signatures.toml and verify all built-in
    /// entries parse correctly.
    #[test]
    fn builtin_signatures_parse() {
        let toml = include_str!("../../../config/signatures.toml");
        let cfg = CarvingConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.signatures.len(), 27, "expected 27 built-in signatures");

        // Spot-check a few well-known magic sequences
        let jpeg = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "jpg")
            .unwrap();
        assert_eq!(
            jpeg.header,
            &[Some(0xFF), Some(0xD8), Some(0xFF)]
        );
        assert_eq!(jpeg.footer, &[0xFF, 0xD9]);

        let png = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "png")
            .unwrap();
        assert_eq!(
            png.header,
            &[
                Some(0x89), Some(0x50), Some(0x4E), Some(0x47),
                Some(0x0D), Some(0x0A), Some(0x1A), Some(0x0A)
            ]
        );

        let sevenz = cfg.signatures.iter().find(|s| s.extension == "7z").unwrap();
        assert_eq!(
            sevenz.header,
            &[Some(0x37), Some(0x7A), Some(0xBC), Some(0xAF), Some(0x27), Some(0x1C)]
        );

        // AVI and WAV must have wildcard bytes at positions 4-7 (the RIFF size field).
        let avi = cfg.signatures.iter().find(|s| s.extension == "avi").unwrap();
        assert_eq!(avi.header[4], None, "AVI header byte 4 should be wildcard");
        assert!(avi.size_hint.is_some(), "AVI should have a size_hint");

        let wav = cfg.signatures.iter().find(|s| s.extension == "wav").unwrap();
        assert_eq!(wav.header[4], None, "WAV header byte 4 should be wildcard");
        assert!(wav.size_hint.is_some(), "WAV should have a size_hint");
    }

    /// End-to-end test: embed a JPEG and PNG marker in a small device image,
    /// scan with the real signatures.toml, then extract the JPEG.
    #[test]
    fn end_to_end_scan_and_extract() {
        let toml = include_str!("../../../config/signatures.toml");
        let mut cfg = CarvingConfig::from_toml_str(toml).unwrap();
        cfg.scan_chunk_size = 512; // small chunks to stress-test boundaries

        let mut data = vec![0u8; 4096];
        // JPEG at offset 0
        data[0..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        data[20..22].copy_from_slice(&[0xFF, 0xD9]); // JPEG footer
                                                     // PNG at offset 1024
        data[1024..1032].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        let dev: Arc<dyn BlockDevice> = Arc::new(MockBlockDevice::new(data, 512));
        let carver = Carver::new(Arc::clone(&dev), cfg);

        let hits = carver.scan().unwrap();
        let extensions: Vec<&str> = hits
            .iter()
            .map(|h| h.signature.extension.as_str())
            .collect();
        assert!(
            extensions.contains(&"jpg"),
            "JPEG not found: {extensions:?}"
        );
        assert!(extensions.contains(&"png"), "PNG not found: {extensions:?}");

        // Extract the JPEG hit — should stop at footer (bytes 0..=21).
        let jpeg_hit = hits
            .iter()
            .find(|h| h.signature.extension == "jpg")
            .unwrap();
        let mut extracted = Vec::new();
        let written = carver.extract(jpeg_hit, &mut extracted).unwrap();
        assert_eq!(
            written, 22,
            "JPEG extract should include footer: {written} bytes"
        );
        assert_eq!(&extracted[20..22], &[0xFF, 0xD9]);
    }
}
