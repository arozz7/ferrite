//! Drive profile — aggregate filesystem entries into per-category statistics.
//!
//! Entirely pure: no I/O.  Call [`build_profile`] with the output of
//! [`FilesystemParser::enumerate_files`] to obtain a [`DriveProfile`], then
//! optionally call [`infer_drive_type`] to get a human-readable label.

use std::collections::HashMap;

use crate::{FileEntry, FilesystemType};

// ── Category ──────────────────────────────────────────────────────────────────

/// High-level file category derived from a file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileCategory {
    Images,
    RawPhoto,
    Video,
    Audio,
    Archive,
    Document,
    System,
    Database,
    Other,
}

impl FileCategory {
    /// Short display label used in the TUI table.
    pub fn label(self) -> &'static str {
        match self {
            Self::Images => "Images",
            Self::RawPhoto => "RAW Photos",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Archive => "Archives",
            Self::Document => "Documents",
            Self::System => "System",
            Self::Database => "Database",
            Self::Other => "Other",
        }
    }
}

/// Map a lowercase file extension to a [`FileCategory`].
pub fn ext_to_category(ext: &str) -> FileCategory {
    match ext {
        // Images (raster)
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "psd" | "ico"
        | "psb" | "exr" | "xcf" | "jp2" | "pcx" | "bpg" | "dpx" | "heic" | "heif" => {
            FileCategory::Images
        }
        // RAW camera formats
        "arw" | "cr2" | "nef" | "rw2" | "raf" | "orf" | "pef" | "cr3" | "sr2" | "dcr" | "crw"
        | "mrw" | "x3f" => FileCategory::RawPhoto,
        // Video
        "mp4" | "mov" | "m4v" | "3gp" | "avi" | "mkv" | "webm" | "wmv" | "flv" | "mpg" | "mpeg"
        | "rm" | "swf" | "ts" | "m2ts" | "wtv" | "vob" => FileCategory::Video,
        // Audio
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "midi" | "mid" | "aiff" | "wv" | "ape" | "au"
        | "aac" | "wma" => FileCategory::Audio,
        // Archives & disk images
        "zip" | "rar" | "7z" | "gz" | "xz" | "bz2" | "iso" | "tar" | "jar" | "lzh" | "aff"
        | "par2" | "cab" | "dmg" => FileCategory::Archive,
        // Documents & office
        "pdf" | "xml" | "html" | "htm" | "rtf" | "vcf" | "ics" | "eml" | "epub" | "cdr" | "ttf"
        | "woff" | "woff2" | "otf" | "chm" | "djvu" | "pem" | "doc" | "docx" | "xls" | "xlsx"
        | "ppt" | "pptx" | "txt" | "csv" | "md" | "odt" | "ods" | "odp" => FileCategory::Document,
        // System & executables
        "dll" | "exe" | "sys" | "drv" | "lnk" | "pf" | "evtx" | "evt" | "reg" | "ini" | "cfg"
        | "bat" | "ps1" | "sh" | "so" | "dylib" | "msi" | "inf" => FileCategory::System,
        // Databases
        "sql" | "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "ldf" | "mdf" => {
            FileCategory::Database
        }
        _ => FileCategory::Other,
    }
}

// ── Stats ─────────────────────────────────────────────────────────────────────

/// File counts and byte totals for one [`FileCategory`].
#[derive(Debug, Clone, Default)]
pub struct CategoryStats {
    pub active_count: u64,
    pub deleted_count: u64,
    pub active_bytes: u64,
    pub deleted_bytes: u64,
}

impl CategoryStats {
    pub fn total_count(&self) -> u64 {
        self.active_count + self.deleted_count
    }

    pub fn total_bytes(&self) -> u64 {
        self.active_bytes + self.deleted_bytes
    }
}

// ── Profile ───────────────────────────────────────────────────────────────────

/// Aggregated statistics for every file category found on a volume.
#[derive(Debug, Clone)]
pub struct DriveProfile {
    /// Per-category breakdown.  Only categories with ≥1 file are present.
    pub stats: HashMap<FileCategory, CategoryStats>,
    /// Number of active (non-deleted) files.
    pub total_active: u64,
    /// Number of deleted files.
    pub total_deleted: u64,
    /// Total bytes across all files (active + deleted).
    pub total_bytes: u64,
    /// Filesystem type the profile was built from.
    pub fs_type: FilesystemType,
}

/// Build a [`DriveProfile`] from a flat list of [`FileEntry`] values.
///
/// Directories are skipped.  Extensions are lowercased before lookup.
pub fn build_profile(files: &[FileEntry], fs_type: FilesystemType) -> DriveProfile {
    let mut stats: HashMap<FileCategory, CategoryStats> = HashMap::new();
    let mut total_active = 0u64;
    let mut total_deleted = 0u64;
    let mut total_bytes = 0u64;

    for file in files {
        if file.is_dir {
            continue;
        }

        let ext = file
            .name
            .rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let cat = ext_to_category(&ext);
        let entry = stats.entry(cat).or_default();

        if file.is_deleted {
            entry.deleted_count += 1;
            entry.deleted_bytes += file.size;
            total_deleted += 1;
        } else {
            entry.active_count += 1;
            entry.active_bytes += file.size;
            total_active += 1;
        }
        total_bytes += file.size;
    }

    DriveProfile {
        stats,
        total_active,
        total_deleted,
        total_bytes,
        fs_type,
    }
}

/// Return a short human-readable label describing the likely purpose of the
/// drive based on the category distribution.
pub fn infer_drive_type(profile: &DriveProfile) -> &'static str {
    let total = profile.total_active + profile.total_deleted;
    if total == 0 {
        return "Empty / Unreadable";
    }

    let pct = |cat: FileCategory| -> f64 {
        profile
            .stats
            .get(&cat)
            .map(|s| s.total_count() as f64 / total as f64 * 100.0)
            .unwrap_or(0.0)
    };

    let images = pct(FileCategory::Images) + pct(FileCategory::RawPhoto);
    let video = pct(FileCategory::Video);
    let audio = pct(FileCategory::Audio);
    let docs = pct(FileCategory::Document);
    let system = pct(FileCategory::System);
    let media = images + video + audio;

    if images > 50.0 {
        return "Camera / Photo Storage";
    }
    if video > 40.0 {
        return "Media Server / Video Library";
    }
    if audio > 40.0 {
        return "Music Library";
    }
    if system > 40.0 {
        return "Windows System Drive";
    }
    if docs > 40.0 {
        return "Personal Workstation / Office PC";
    }
    if media > 50.0 {
        return "General Media Storage";
    }
    "General-Purpose Storage"
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecoveryChance;

    fn make_entry(name: &str, size: u64, is_deleted: bool) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: format!("/{name}"),
            size,
            is_dir: false,
            is_deleted,
            created: None,
            modified: None,
            first_cluster: None,
            mft_record: None,
            inode_number: None,
            data_byte_offset: None,
            recovery_chance: RecoveryChance::Unknown,
        }
    }

    fn make_dir(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: format!("/{name}"),
            size: 0,
            is_dir: true,
            is_deleted: false,
            created: None,
            modified: None,
            first_cluster: None,
            mft_record: None,
            inode_number: None,
            data_byte_offset: None,
            recovery_chance: RecoveryChance::Unknown,
        }
    }

    #[test]
    fn ext_to_category_images() {
        assert_eq!(ext_to_category("jpg"), FileCategory::Images);
        assert_eq!(ext_to_category("png"), FileCategory::Images);
        assert_eq!(ext_to_category("webp"), FileCategory::Images);
    }

    #[test]
    fn ext_to_category_raw() {
        assert_eq!(ext_to_category("cr2"), FileCategory::RawPhoto);
        assert_eq!(ext_to_category("arw"), FileCategory::RawPhoto);
    }

    #[test]
    fn ext_to_category_audio() {
        assert_eq!(ext_to_category("mp3"), FileCategory::Audio);
        assert_eq!(ext_to_category("flac"), FileCategory::Audio);
    }

    #[test]
    fn ext_to_category_system() {
        assert_eq!(ext_to_category("dll"), FileCategory::System);
        assert_eq!(ext_to_category("exe"), FileCategory::System);
    }

    #[test]
    fn ext_to_category_unknown_is_other() {
        assert_eq!(ext_to_category("xyz"), FileCategory::Other);
        assert_eq!(ext_to_category(""), FileCategory::Other);
        assert_eq!(ext_to_category("ferrite"), FileCategory::Other);
    }

    #[test]
    fn build_profile_empty_input() {
        let profile = build_profile(&[], FilesystemType::Ntfs);
        assert_eq!(profile.total_active, 0);
        assert_eq!(profile.total_deleted, 0);
        assert_eq!(profile.total_bytes, 0);
        assert!(profile.stats.is_empty());
    }

    #[test]
    fn build_profile_skips_directories() {
        let files = vec![make_dir("Documents"), make_entry("photo.jpg", 1024, false)];
        let profile = build_profile(&files, FilesystemType::Ntfs);
        assert_eq!(profile.total_active, 1);
        assert_eq!(profile.total_deleted, 0);
        assert_eq!(profile.stats.len(), 1);
    }

    #[test]
    fn build_profile_counts_active_and_deleted() {
        let files = vec![
            make_entry("photo.jpg", 2_000, false),
            make_entry("old.jpg", 1_000, true),
            make_entry("song.mp3", 5_000, false),
        ];
        let profile = build_profile(&files, FilesystemType::Ntfs);
        assert_eq!(profile.total_active, 2);
        assert_eq!(profile.total_deleted, 1);
        assert_eq!(profile.total_bytes, 8_000);

        let img = profile.stats.get(&FileCategory::Images).unwrap();
        assert_eq!(img.active_count, 1);
        assert_eq!(img.deleted_count, 1);
        assert_eq!(img.active_bytes, 2_000);
        assert_eq!(img.deleted_bytes, 1_000);

        let aud = profile.stats.get(&FileCategory::Audio).unwrap();
        assert_eq!(aud.active_count, 1);
        assert_eq!(aud.deleted_count, 0);
    }

    #[test]
    fn build_profile_extension_case_insensitive() {
        let files = vec![make_entry("photo.JPG", 512, false)];
        let profile = build_profile(&files, FilesystemType::Ntfs);
        assert!(profile.stats.contains_key(&FileCategory::Images));
    }

    #[test]
    fn category_stats_totals() {
        let s = CategoryStats {
            active_count: 3,
            deleted_count: 2,
            active_bytes: 300,
            deleted_bytes: 200,
        };
        assert_eq!(s.total_count(), 5);
        assert_eq!(s.total_bytes(), 500);
    }

    #[test]
    fn infer_drive_type_empty() {
        let profile = build_profile(&[], FilesystemType::Ntfs);
        assert_eq!(infer_drive_type(&profile), "Empty / Unreadable");
    }

    #[test]
    fn infer_drive_type_photo_heavy() {
        // 80 JPEGs → should be "Camera / Photo Storage"
        let files: Vec<FileEntry> = (0..80)
            .map(|i| make_entry(&format!("img{i}.jpg"), 1024, false))
            .chain((0..20).map(|i| make_entry(&format!("doc{i}.pdf"), 512, false)))
            .collect();
        let profile = build_profile(&files, FilesystemType::Fat32);
        assert_eq!(infer_drive_type(&profile), "Camera / Photo Storage");
    }

    #[test]
    fn infer_drive_type_system_heavy() {
        let files: Vec<FileEntry> = (0..60)
            .map(|i| make_entry(&format!("lib{i}.dll"), 4096, false))
            .chain((0..40).map(|i| make_entry(&format!("app{i}.exe"), 8192, false)))
            .collect();
        let profile = build_profile(&files, FilesystemType::Ntfs);
        assert_eq!(infer_drive_type(&profile), "Windows System Drive");
    }
}
