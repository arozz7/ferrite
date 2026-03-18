//! CSV export for artifact scan results.

use std::fmt::Write as FmtWrite;
use std::io;

use crate::scanner::ArtifactHit;

/// Write `hits` to a CSV file at `path`.
///
/// Format: `byte_offset,kind,value` with a header row.  Values are
/// double-quote escaped (commas and quotes within values are handled).
pub fn write_csv(path: &str, hits: &[ArtifactHit]) -> io::Result<()> {
    let mut out = String::with_capacity(hits.len() * 64);
    out.push_str("byte_offset,kind,value\n");
    for hit in hits {
        let escaped_value = hit.value.replace('"', "\"\"");
        writeln!(
            out,
            "{},{},\"{}\"",
            hit.byte_offset,
            hit.kind.label(),
            escaped_value
        )
        .expect("write to String never fails");
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::ArtifactKind;

    fn make_hit(kind: ArtifactKind, offset: u64, value: &str) -> ArtifactHit {
        ArtifactHit {
            kind,
            byte_offset: offset,
            value: value.to_string(),
        }
    }

    #[test]
    fn write_csv_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let path_str = path.to_str().unwrap();

        let hits = vec![
            make_hit(ArtifactKind::Email, 100, "alice@example.com"),
            make_hit(ArtifactKind::Url, 200, "https://example.com"),
            make_hit(ArtifactKind::CreditCard, 300, "****-****-****-1234"),
        ];

        write_csv(path_str, &hits).unwrap();
        let content = std::fs::read_to_string(path_str).unwrap();
        assert!(content.starts_with("byte_offset,kind,value\n"));
        assert!(content.contains("alice@example.com"));
        assert!(content.contains("https://example.com"));
        assert!(content.contains("****-****-****-1234"));
    }

    #[test]
    fn csv_escapes_quotes_in_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.csv");
        let hits = vec![make_hit(ArtifactKind::Email, 0, r#"he said "hi""#)];
        write_csv(path.to_str().unwrap(), &hits).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#"he said ""hi"""#));
    }
}
