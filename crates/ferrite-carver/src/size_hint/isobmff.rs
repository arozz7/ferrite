//! ISO Base Media File Format (ISOBMFF) box walker size-hint handler.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

pub(super) fn isobmff_hint(device: &dyn BlockDevice, file_offset: u64) -> Option<u64> {
    const MAX_BOXES: u32 = 2_000;
    let device_size = device.size();
    let mut pos = file_offset;
    let mut total: u64 = 0;

    for _ in 0..MAX_BOXES {
        if pos + 8 > device_size {
            break;
        }
        let hdr = read_bytes_clamped(device, pos, 8).ok()?;
        if hdr.len() < 8 {
            break;
        }
        let box_size_raw = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let box_type = [hdr[4], hdr[5], hdr[6], hdr[7]];

        if !box_type.iter().all(|b| (0x20..=0x7E).contains(b)) {
            break;
        }

        let actual_size: u64 = match box_size_raw {
            0 => break,
            1 => {
                if pos + 16 > device_size {
                    break;
                }
                let ls = read_bytes_clamped(device, pos + 8, 8).ok()?;
                if ls.len() < 8 {
                    break;
                }
                let sz = u64::from_be_bytes(ls[..8].try_into().ok()?);
                if sz < 16 {
                    break;
                }
                sz
            }
            n if n < 8 => break,
            n => n as u64,
        };

        total = total.saturating_add(actual_size);
        pos = pos.saturating_add(actual_size);
    }

    if total > 0 {
        Some(total)
    } else {
        None
    }
}
