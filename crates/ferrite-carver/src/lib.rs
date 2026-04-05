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

mod carver_io;
mod error;
pub mod post_validate;
mod pre_validate;
mod scan_search;
mod scanner;
mod signature;
mod size_hint;

pub use error::{CarveError, Result};
pub use post_validate::CarveQuality;
pub use pre_validate::PreValidate;
pub use scanner::{CarveHit, Carver, ScanProgress};
pub use signature::{parse_hex, parse_hex_pattern, CarvingConfig, Signature, SizeHint};

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
        assert_eq!(
            cfg.signatures.len(),
            140,
            "expected 140 built-in signatures"
        );

        // All three JPEG variants must be present with 4-byte headers and Jpeg size hint.
        let jpeg_jfif = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "jpg" && s.header.get(3) == Some(&Some(0xE0)))
            .expect("JPEG JFIF (FF D8 FF E0) not found");
        assert_eq!(
            jpeg_jfif.header,
            &[Some(0xFF), Some(0xD8), Some(0xFF), Some(0xE0)]
        );
        assert!(
            jpeg_jfif.footer.is_empty(),
            "JPEG JFIF footer must be empty (walker-based)"
        );
        assert_eq!(
            jpeg_jfif.size_hint,
            Some(super::SizeHint::Jpeg),
            "JPEG JFIF must use Jpeg size hint"
        );

        let jpeg_exif = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "jpg" && s.header.get(3) == Some(&Some(0xE1)))
            .expect("JPEG Exif (FF D8 FF E1) not found");
        assert_eq!(
            jpeg_exif.header,
            &[Some(0xFF), Some(0xD8), Some(0xFF), Some(0xE1)]
        );
        assert!(
            jpeg_exif.footer.is_empty(),
            "JPEG Exif footer must be empty (walker-based)"
        );
        assert_eq!(
            jpeg_exif.size_hint,
            Some(super::SizeHint::Jpeg),
            "JPEG Exif must use Jpeg size hint"
        );

        let jpeg_dqt = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "jpg" && s.header.get(3) == Some(&Some(0xDB)))
            .expect("JPEG Raw/DQT (FF D8 FF DB) not found");
        assert_eq!(
            jpeg_dqt.header,
            &[Some(0xFF), Some(0xD8), Some(0xFF), Some(0xDB)]
        );
        assert!(
            jpeg_dqt.footer.is_empty(),
            "JPEG DQT footer must be empty (walker-based)"
        );
        assert_eq!(
            jpeg_dqt.size_hint,
            Some(super::SizeHint::Jpeg),
            "JPEG DQT must use Jpeg size hint"
        );

        // PDF must use footer_last mode.
        let pdf = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "pdf")
            .unwrap();
        assert!(pdf.footer_last, "PDF should have footer_last = true");

        let png = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "png")
            .unwrap();
        assert_eq!(
            png.header,
            &[
                Some(0x89),
                Some(0x50),
                Some(0x4E),
                Some(0x47),
                Some(0x0D),
                Some(0x0A),
                Some(0x1A),
                Some(0x0A)
            ]
        );

        let sevenz = cfg.signatures.iter().find(|s| s.extension == "7z").unwrap();
        assert_eq!(
            sevenz.header,
            &[
                Some(0x37),
                Some(0x7A),
                Some(0xBC),
                Some(0xAF),
                Some(0x27),
                Some(0x1C)
            ]
        );

        // AVI and WAV must have wildcard bytes at positions 4-7 (the RIFF size field).
        let avi = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "avi")
            .unwrap();
        assert_eq!(avi.header[4], None, "AVI header byte 4 should be wildcard");
        assert!(avi.size_hint.is_some(), "AVI should have a size_hint");

        let wav = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "wav")
            .unwrap();
        assert_eq!(wav.header[4], None, "WAV header byte 4 should be wildcard");
        assert!(wav.size_hint.is_some(), "WAV should have a size_hint");

        // OLE2 must use the Ole2 size hint variant.
        let ole = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "ole")
            .unwrap();
        assert_eq!(
            ole.size_hint,
            Some(super::SizeHint::Ole2),
            "OLE2 should use SizeHint::Ole2"
        );

        // SQLite must use the Sqlite size hint variant.
        let sqlite = cfg.signatures.iter().find(|s| s.extension == "db").unwrap();
        assert_eq!(
            sqlite.size_hint,
            Some(super::SizeHint::Sqlite),
            "SQLite should use SizeHint::Sqlite"
        );

        // 7-Zip must use the SevenZip size hint variant.
        let sevenz = cfg.signatures.iter().find(|s| s.extension == "7z").unwrap();
        assert_eq!(
            sevenz.size_hint,
            Some(super::SizeHint::SevenZip),
            "7-Zip should use SizeHint::SevenZip"
        );

        // EVTX must use LinearScaled with the correct parameters.
        let evtx = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "evtx")
            .unwrap();
        match evtx.size_hint.as_ref().unwrap() {
            super::SizeHint::LinearScaled {
                offset,
                len,
                scale,
                add,
                ..
            } => {
                assert_eq!(*offset, 42, "EVTX size_hint offset");
                assert_eq!(*len, 2, "EVTX size_hint len");
                assert_eq!(*scale, 65536, "EVTX chunk size");
                assert_eq!(*add, 4096, "EVTX header size");
            }
            other => panic!("EVTX should use SizeHint::LinearScaled, got {other:?}"),
        }

        // OGG must use OggStream size hint.
        let ogg = cfg
            .signatures
            .iter()
            .find(|s| s.extension == "ogg")
            .unwrap();
        assert_eq!(
            ogg.size_hint,
            Some(super::SizeHint::OggStream),
            "OGG should use SizeHint::OggStream"
        );
        assert_eq!(
            ogg.header,
            &[Some(0x4F), Some(0x67), Some(0x67), Some(0x53)],
            "OGG header should be 'OggS'"
        );
    }

    /// End-to-end test: embed a JPEG and PNG marker in a small device image,
    /// scan with the real signatures.toml, then extract the JPEG.
    #[test]
    fn end_to_end_scan_and_extract() {
        let toml = include_str!("../../../config/signatures.toml");
        let mut cfg = CarvingConfig::from_toml_str(toml).unwrap();
        cfg.scan_chunk_size = 512; // small chunks to stress-test boundaries

        let mut data = vec![0u8; 8192];

        // Structurally-valid JPEG (Exif) at offset 0.
        //
        // Layout (total = 4096 bytes, matching min_size exactly):
        //   [0..2]     FF D8     SOI
        //   [2..4]     FF E1     APP1 marker
        //   [4..6]     0F F4     APP1 length = 4084 (= 4082 bytes data + 2 for field itself)
        //   [6..10]    "Exif"    pre_validate = "jpeg_exif" requires this at offset 6
        //   [10..4088] 0x00...   rest of APP1 payload — no embedded thumbnail FF D9
        //   [4088..4090] FF DA   SOS marker  (pos after APP1 = 2+2+4084 = 4088 ✓)
        //   [4090..4092] 00 04   SOS length = 4
        //   [4092..4094] 00 00   SOS header data
        //   [4094..4096] FF D9   EOI  (scan_for_eoi finds this at offset 4094)
        data[0..2].copy_from_slice(&[0xFF, 0xD8]); // SOI
        data[2..4].copy_from_slice(&[0xFF, 0xE1]); // APP1 marker
        data[4..6].copy_from_slice(&4084u16.to_be_bytes()); // APP1 length = 4084
        data[6..10].copy_from_slice(b"Exif"); // pre_validate requirement
                                              // data[10..4088] remains 0x00 (APP1 payload — no embedded thumbnail FF D9)
        data[4088..4090].copy_from_slice(&[0xFF, 0xDA]); // SOS marker
        data[4090..4092].copy_from_slice(&4u16.to_be_bytes()); // SOS length = 4
        data[4092..4094].copy_from_slice(&[0x00, 0x00]); // SOS header data
        data[4094..4096].copy_from_slice(&[0xFF, 0xD9]); // EOI

        // PNG at offset 5120 (past the JPEG which ends at 4114)
        data[5120..5128].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        data[5128..5132].copy_from_slice(&13u32.to_be_bytes()); // IHDR length = 13
        data[5132..5136].copy_from_slice(b"IHDR"); // pre_validate requires IHDR first chunk

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

        // Extract the JPEG hit — walker should find the EOI at byte 4114.
        let jpeg_hit = hits
            .iter()
            .find(|h| h.signature.extension == "jpg")
            .unwrap();
        let mut extracted = Vec::new();
        let written = carver.extract(jpeg_hit, &mut extracted).unwrap();
        assert_eq!(
            written, 4096,
            "JPEG segment walker should extract exactly 4096 bytes; got {written}"
        );
        assert_eq!(&extracted[4094..4096], &[0xFF, 0xD9]);
    }
}
