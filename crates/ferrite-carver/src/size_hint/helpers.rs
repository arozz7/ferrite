//! Byte-reading helpers for size-hint implementations.

/// Read a u16 from a 2-byte slice in the specified endianness.
pub(super) fn read_u16(b: &[u8], little_endian: bool) -> u16 {
    if little_endian {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    }
}

/// Read a u32 from a 4-byte slice in the specified endianness.
pub(super) fn read_u32(b: &[u8], little_endian: bool) -> u32 {
    if little_endian {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

/// Read a u64 from an 8-byte slice in the specified endianness.
pub(super) fn read_u64(b: &[u8], little_endian: bool) -> u64 {
    if little_endian {
        u64::from_le_bytes(b[..8].try_into().unwrap())
    } else {
        u64::from_be_bytes(b[..8].try_into().unwrap())
    }
}

/// Read a u16 LE from a 2-byte slice.
pub(super) fn read_u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

/// Read a u32 LE from a 4-byte slice.
pub(super) fn read_u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
