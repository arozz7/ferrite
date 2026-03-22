use serde::{Deserialize, Serialize};

/// Recovery status of a contiguous byte range, matching GNU ddrescue codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockStatus {
    NonTried,   // '?' — not yet attempted
    NonTrimmed, // '*' — read failed; exact bad-sector location unknown
    NonScraped, // '/' — trim found the boundary; scrape not yet attempted
    BadSector,  // '-' — scrape failed; sector is unreadable
    Finished,   // '+' — read and written successfully
}

impl BlockStatus {
    pub fn to_char(self) -> char {
        match self {
            Self::NonTried => '?',
            Self::NonTrimmed => '*',
            Self::NonScraped => '/',
            Self::BadSector => '-',
            Self::Finished => '+',
        }
    }

    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '?' => Some(Self::NonTried),
            '*' => Some(Self::NonTrimmed),
            '/' => Some(Self::NonScraped),
            '-' => Some(Self::BadSector),
            '+' => Some(Self::Finished),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::NonTried => 0,
            Self::NonTrimmed => 1,
            Self::NonScraped => 2,
            Self::BadSector => 3,
            Self::Finished => 4,
        }
    }
}

/// A contiguous byte range with a uniform recovery status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub pos: u64,
    pub size: u64,
    pub status: BlockStatus,
}

impl Block {
    pub fn end(&self) -> u64 {
        self.pos + self.size
    }
}

/// Sorted, non-overlapping, gapless list of blocks covering `[0, device_size)`.
///
/// This is the authoritative state for an imaging session — every byte of the
/// device belongs to exactly one block.
pub struct Mapfile {
    blocks: Vec<Block>,
    device_size: u64,
    /// Byte counts per status (indexed by `BlockStatus::index()`).
    counts: [u64; 5],
}

impl Mapfile {
    // ── Constructors ──────────────────────────────────────────────────────────

    /// Create a fresh mapfile: one `NonTried` block covering the entire device.
    pub fn from_device_size(device_size: u64) -> Self {
        let mut counts = [0u64; 5];
        counts[BlockStatus::NonTried.index()] = device_size;
        Self {
            blocks: vec![Block {
                pos: 0,
                size: device_size,
                status: BlockStatus::NonTried,
            }],
            device_size,
            counts,
        }
    }

    /// Construct from a pre-parsed block list (used by `mapfile_io`).
    pub(crate) fn from_blocks(blocks: Vec<Block>, device_size: u64) -> Self {
        let mut counts = [0u64; 5];
        for b in &blocks {
            counts[b.status.index()] += b.size;
        }
        Self {
            blocks,
            device_size,
            counts,
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    pub fn device_size(&self) -> u64 {
        self.device_size
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Total bytes whose status is `status`.
    pub fn bytes_with_status(&self, status: BlockStatus) -> u64 {
        self.counts[status.index()]
    }

    /// True if any block has the given status.
    pub fn has_status(&self, status: BlockStatus) -> bool {
        self.counts[status.index()] > 0
    }

    /// Iterator over blocks with `status`, in ascending offset order.
    pub fn blocks_with_status(&self, status: BlockStatus) -> impl Iterator<Item = Block> + '_ {
        self.blocks
            .iter()
            .copied()
            .filter(move |b| b.status == status)
    }

    /// The status of the byte at `pos`. O(log n) binary search.
    pub fn status_at(&self, pos: u64) -> Option<BlockStatus> {
        if pos >= self.device_size {
            return None;
        }
        // Find the last block whose .pos <= pos.
        let idx = self.blocks.partition_point(|b| b.pos <= pos);
        if idx == 0 {
            return None;
        }
        let block = &self.blocks[idx - 1];
        if pos < block.end() {
            Some(block.status)
        } else {
            None
        }
    }

    // ── Mutations ─────────────────────────────────────────────────────────────

    /// Update the status of all bytes in `[pos, pos+size)`.
    ///
    /// Splits existing blocks at the boundaries if necessary, replaces the
    /// covered blocks with a single block of `status`, then merges any newly
    /// adjacent blocks that share a status.
    pub fn update_range(&mut self, pos: u64, size: u64, status: BlockStatus) {
        if size == 0 {
            return;
        }
        let end = pos + size;

        // First block whose end() > pos (overlaps or starts after pos).
        let start_idx = self.blocks.partition_point(|b| b.end() <= pos);
        // First block whose pos >= end (starts at or after our range's end).
        let end_idx = self.blocks.partition_point(|b| b.pos < end);

        let mut replacements: Vec<Block> = Vec::with_capacity(3);

        if start_idx < end_idx {
            let first = self.blocks[start_idx];
            let last = self.blocks[end_idx - 1];

            // Prefix: the portion of the first overlapping block before `pos`.
            if first.pos < pos {
                replacements.push(Block {
                    pos: first.pos,
                    size: pos - first.pos,
                    status: first.status,
                });
            }
            // The updated region.
            replacements.push(Block { pos, size, status });
            // Suffix: the portion of the last overlapping block after `end`.
            if last.end() > end {
                replacements.push(Block {
                    pos: end,
                    size: last.end() - end,
                    status: last.status,
                });
            }
        } else {
            // No existing blocks overlap — insert into a gap (shouldn't happen
            // for a well-formed mapfile, but handle gracefully).
            replacements.push(Block { pos, size, status });
        }

        self.blocks.splice(start_idx..end_idx, replacements);
        self.merge_and_recount();
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    /// Merge contiguous adjacent blocks that share the same status, and recompute
    /// byte counts — all in a single O(n) pass to avoid scanning the block list twice.
    fn merge_and_recount(&mut self) {
        let mut counts = [0u64; 5];
        if self.blocks.len() <= 1 {
            for b in &self.blocks {
                counts[b.status.index()] += b.size;
            }
            self.counts = counts;
            return;
        }

        let mut merged: Vec<Block> = Vec::with_capacity(self.blocks.len());
        for block in self.blocks.drain(..) {
            match merged.last_mut() {
                Some(last) if last.status == block.status && last.end() == block.pos => {
                    last.size += block.size;
                }
                _ => merged.push(block),
            }
        }
        for b in &merged {
            counts[b.status.index()] += b.size;
        }
        self.blocks = merged;
        self.counts = counts;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mf(size: u64) -> Mapfile {
        Mapfile::from_device_size(size)
    }

    #[test]
    fn fresh_mapfile_single_nontried_block() {
        let m = mf(4096);
        assert_eq!(m.blocks().len(), 1);
        assert_eq!(m.blocks()[0].status, BlockStatus::NonTried);
        assert_eq!(m.bytes_with_status(BlockStatus::NonTried), 4096);
        assert_eq!(m.bytes_with_status(BlockStatus::Finished), 0);
    }

    #[test]
    fn update_full_range_changes_status() {
        let mut m = mf(512);
        m.update_range(0, 512, BlockStatus::Finished);
        assert_eq!(m.blocks().len(), 1);
        assert_eq!(m.blocks()[0].status, BlockStatus::Finished);
        assert_eq!(m.bytes_with_status(BlockStatus::Finished), 512);
        assert_eq!(m.bytes_with_status(BlockStatus::NonTried), 0);
    }

    #[test]
    fn update_middle_splits_into_three() {
        let mut m = mf(1024);
        m.update_range(256, 512, BlockStatus::Finished);
        let blocks = m.blocks();
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0],
            Block {
                pos: 0,
                size: 256,
                status: BlockStatus::NonTried
            }
        );
        assert_eq!(
            blocks[1],
            Block {
                pos: 256,
                size: 512,
                status: BlockStatus::Finished
            }
        );
        assert_eq!(
            blocks[2],
            Block {
                pos: 768,
                size: 256,
                status: BlockStatus::NonTried
            }
        );
    }

    #[test]
    fn update_at_start_splits_into_two() {
        let mut m = mf(1024);
        m.update_range(0, 512, BlockStatus::Finished);
        assert_eq!(m.blocks().len(), 2);
        assert_eq!(m.blocks()[0].status, BlockStatus::Finished);
        assert_eq!(m.blocks()[1].status, BlockStatus::NonTried);
    }

    #[test]
    fn update_at_end_splits_into_two() {
        let mut m = mf(1024);
        m.update_range(512, 512, BlockStatus::Finished);
        assert_eq!(m.blocks().len(), 2);
        assert_eq!(m.blocks()[0].status, BlockStatus::NonTried);
        assert_eq!(m.blocks()[1].status, BlockStatus::Finished);
    }

    #[test]
    fn adjacent_same_status_merges() {
        let mut m = mf(1024);
        m.update_range(0, 512, BlockStatus::Finished);
        m.update_range(512, 512, BlockStatus::Finished);
        assert_eq!(m.blocks().len(), 1);
        assert_eq!(m.bytes_with_status(BlockStatus::Finished), 1024);
    }

    #[test]
    fn different_status_no_merge() {
        let mut m = mf(1024);
        m.update_range(0, 512, BlockStatus::Finished);
        m.update_range(512, 512, BlockStatus::BadSector);
        assert_eq!(m.blocks().len(), 2);
    }

    #[test]
    fn status_at_correct() {
        let mut m = mf(1024);
        m.update_range(256, 512, BlockStatus::Finished);
        assert_eq!(m.status_at(0), Some(BlockStatus::NonTried));
        assert_eq!(m.status_at(255), Some(BlockStatus::NonTried));
        assert_eq!(m.status_at(256), Some(BlockStatus::Finished));
        assert_eq!(m.status_at(767), Some(BlockStatus::Finished));
        assert_eq!(m.status_at(768), Some(BlockStatus::NonTried));
        assert_eq!(m.status_at(1024), None);
    }

    #[test]
    fn has_status() {
        let mut m = mf(512);
        assert!(m.has_status(BlockStatus::NonTried));
        assert!(!m.has_status(BlockStatus::BadSector));
        m.update_range(0, 512, BlockStatus::Finished);
        assert!(!m.has_status(BlockStatus::NonTried));
    }

    #[test]
    fn counts_sum_to_device_size() {
        let mut m = mf(4096);
        m.update_range(0, 512, BlockStatus::Finished);
        m.update_range(512, 512, BlockStatus::BadSector);
        let total: u64 = [
            BlockStatus::NonTried,
            BlockStatus::NonTrimmed,
            BlockStatus::NonScraped,
            BlockStatus::BadSector,
            BlockStatus::Finished,
        ]
        .iter()
        .map(|&s| m.bytes_with_status(s))
        .sum();
        assert_eq!(total, 4096);
    }

    #[test]
    fn update_across_multiple_existing_blocks() {
        let mut m = mf(2048);
        // Create alternating blocks: Finished / NonTried / Finished
        m.update_range(0, 512, BlockStatus::Finished);
        m.update_range(1024, 512, BlockStatus::Finished);
        // Now overwrite the whole device as NonTrimmed
        m.update_range(0, 2048, BlockStatus::NonTrimmed);
        assert_eq!(m.blocks().len(), 1);
        assert_eq!(m.bytes_with_status(BlockStatus::NonTrimmed), 2048);
    }
}
