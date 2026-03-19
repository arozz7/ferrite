//! MPEG-TS / M2TS stream walker size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// Walk an MPEG-TS or Blu-ray M2TS stream forward from `file_offset`,
/// checking that byte `ts_offset` within every `stride`-byte packet equals
/// the TS sync byte `0x47`.
///
/// Returns the total byte length of the contiguous run of valid packets, or
/// `None` if no valid packets are found at all.
pub(super) fn mpeg_ts_size_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    ts_offset: u8,
    stride: u16,
    max_size: u64,
) -> Option<u64> {
    const MAX_INVALID_RUN: u32 = 10;
    const PKTS_PER_READ: u64 = 1024;

    let stride = stride as u64;
    let ts_off = ts_offset as u64;
    let device_size = device.size();
    let scan_end = file_offset.saturating_add(max_size).min(device_size);

    let mut pkt_abs = file_offset;
    let mut last_valid_end: u64 = 0;
    let mut found_any = false;
    let mut invalid_run: u32 = 0;

    'outer: while pkt_abs < scan_end {
        let read_len = (PKTS_PER_READ * stride).min(scan_end - pkt_abs) as usize;
        let data = match read_bytes_clamped(device, pkt_abs, read_len) {
            Ok(d) if !d.is_empty() => d,
            _ => break,
        };

        let mut off = 0usize;
        while off + (ts_off as usize) < data.len() {
            if data[off + (ts_off as usize)] == 0x47 {
                last_valid_end = (pkt_abs - file_offset) + off as u64 + stride;
                found_any = true;
                invalid_run = 0;
            } else {
                invalid_run += 1;
                if invalid_run >= MAX_INVALID_RUN {
                    break 'outer;
                }
            }
            off += stride as usize;
        }

        if data.len() < read_len {
            break;
        }
        pkt_abs += read_len as u64;
    }

    if found_any {
        Some(last_valid_end)
    } else {
        None
    }
}
