//! Linear and LinearScaled size-hint handlers.

use ferrite_blockdev::BlockDevice;

use crate::carver_io::read_bytes_clamped;

/// `total_size = parse(data[offset..offset+len]) + add`
pub(super) fn linear_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    offset: usize,
    len: u8,
    little_endian: bool,
    add: u64,
) -> Option<u64> {
    let field_offset = file_offset + offset as u64;
    let bytes = read_bytes_clamped(device, field_offset, len as usize).ok()?;
    if bytes.len() < len as usize {
        return None;
    }
    let value: u64 = match len {
        2 => {
            let arr: [u8; 2] = bytes[..2].try_into().ok()?;
            if little_endian {
                u16::from_le_bytes(arr) as u64
            } else {
                u16::from_be_bytes(arr) as u64
            }
        }
        4 => {
            let arr: [u8; 4] = bytes[..4].try_into().ok()?;
            if little_endian {
                u32::from_le_bytes(arr) as u64
            } else {
                u32::from_be_bytes(arr) as u64
            }
        }
        8 => {
            let arr: [u8; 8] = bytes[..8].try_into().ok()?;
            if little_endian {
                u64::from_le_bytes(arr)
            } else {
                u64::from_be_bytes(arr)
            }
        }
        _ => return None,
    };
    Some(value.saturating_add(add))
}

/// `total_size = parse(data[offset..offset+len]) × scale + add`
pub(super) fn linear_scaled_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    offset: usize,
    len: u8,
    little_endian: bool,
    scale: u64,
    add: u64,
) -> Option<u64> {
    let field_offset = file_offset + offset as u64;
    let bytes = read_bytes_clamped(device, field_offset, len as usize).ok()?;
    if bytes.len() < len as usize {
        return None;
    }
    let value: u64 = match len {
        2 => {
            let arr: [u8; 2] = bytes[..2].try_into().ok()?;
            if little_endian {
                u16::from_le_bytes(arr) as u64
            } else {
                u16::from_be_bytes(arr) as u64
            }
        }
        4 => {
            let arr: [u8; 4] = bytes[..4].try_into().ok()?;
            if little_endian {
                u32::from_le_bytes(arr) as u64
            } else {
                u32::from_be_bytes(arr) as u64
            }
        }
        8 => {
            let arr: [u8; 8] = bytes[..8].try_into().ok()?;
            if little_endian {
                u64::from_le_bytes(arr)
            } else {
                u64::from_be_bytes(arr)
            }
        }
        _ => return None,
    };
    Some(value.saturating_mul(scale).saturating_add(add))
}
