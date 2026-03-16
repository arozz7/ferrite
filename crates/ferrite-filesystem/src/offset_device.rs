//! `OffsetDevice` — wraps a `BlockDevice` at a fixed byte offset.
//!
//! Used internally so filesystem parsers (which always read from offset 0 of
//! their "volume") can transparently operate on a partition that starts
//! somewhere in the middle of a whole-disk device.

use std::sync::Arc;

use ferrite_blockdev::{AlignedBuffer, BlockDevice};
use ferrite_core::types::DeviceInfo;

pub(crate) struct OffsetDevice {
    pub(crate) inner: Arc<dyn BlockDevice>,
    pub(crate) offset: u64,
}

impl BlockDevice for OffsetDevice {
    fn read_at(&self, offset: u64, buf: &mut AlignedBuffer) -> ferrite_blockdev::Result<usize> {
        self.inner.read_at(self.offset + offset, buf)
    }

    fn size(&self) -> u64 {
        self.inner.size().saturating_sub(self.offset)
    }

    fn sector_size(&self) -> u32 {
        self.inner.sector_size()
    }

    fn device_info(&self) -> &DeviceInfo {
        self.inner.device_info()
    }
}
