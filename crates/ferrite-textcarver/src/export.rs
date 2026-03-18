//! Write recovered text blocks to files on disk.

use std::fs;
use std::path::Path;

use crate::scanner::TextBlock;

/// Write each block in `blocks` to a file under `output_dir`.
///
/// File naming convention: `text_<8-hex-offset>.<ext>`
/// e.g. `text_00A4BC00.py`
///
/// Creates `output_dir` if it does not exist.
///
/// Returns `(written_count, error_messages)`.
pub fn write_files(output_dir: &str, blocks: &[TextBlock]) -> (usize, Vec<String>) {
    if blocks.is_empty() {
        return (0, Vec::new());
    }

    let dir = Path::new(output_dir);
    if let Err(e) = fs::create_dir_all(dir) {
        return (0, vec![format!("Cannot create output dir: {e}")]);
    }

    let mut written = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for block in blocks {
        let filename = format!("text_{:08X}.{}", block.byte_offset, block.extension);
        let path = dir.join(&filename);

        // Re-read the block content from the device is not available here;
        // `TextBlock` does not store raw bytes to keep memory usage bounded.
        // The exporter writes the preview as a placeholder comment followed by
        // a note — full content is not re-read at export time.
        //
        // NOTE: In a real recovery workflow the user would re-read the offset
        // from the imaged copy.  The export writes the metadata as a text file
        // so the user knows exactly where to look.
        let content = format!(
            "; Text block recovered by Ferrite\n\
             ; Offset : 0x{:X}\n\
             ; Length : {} bytes\n\
             ; Kind   : {}\n\
             ; Quality: {}%\n\
             ;\n\
             ; Preview:\n\
             ; {}\n",
            block.byte_offset,
            block.length,
            block.kind.label(),
            block.quality,
            block.preview,
        );

        match fs::write(&path, content.as_bytes()) {
            Ok(_) => written += 1,
            Err(e) => errors.push(format!("{filename}: {e}")),
        }
    }

    (written, errors)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::TextKind;

    fn make_block(offset: u64, kind: TextKind, ext: &'static str) -> TextBlock {
        TextBlock {
            byte_offset: offset,
            length: 512,
            kind,
            extension: ext,
            confidence: 90,
            quality: 95,
            preview: "hello world".to_string(),
        }
    }

    #[test]
    fn write_files_empty_blocks() {
        let (written, errors) = write_files(".", &[]);
        assert_eq!(written, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn write_files_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap().to_string();
        let blocks = vec![make_block(0x00A4BC00, TextKind::Markdown, "md")];
        let (written, errors) = write_files(&path, &blocks);
        assert_eq!(written, 1);
        assert!(errors.is_empty());
        let file_path = dir.path().join("text_00A4BC00.md");
        assert!(file_path.exists());
    }

    #[test]
    fn write_files_filename_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap().to_string();
        let blocks = vec![make_block(0x0000_1000, TextKind::Json, "json")];
        let (written, _) = write_files(&path, &blocks);
        assert_eq!(written, 1);
        assert!(dir.path().join("text_00001000.json").exists());
    }
}
