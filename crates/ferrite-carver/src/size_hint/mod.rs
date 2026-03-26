//! Size-hint readers: derive the true file size from embedded header fields.
//!
//! Each [`SizeHint`] variant has a corresponding arm here that reads the
//! minimum number of bytes from the device and returns the implied total file
//! size, or `None` when the header is malformed or a read fails (the caller
//! falls back to `max_size`).

mod asf;
mod au;
mod ebml;
mod elf;
mod gif;
pub(crate) mod helpers;
mod iso9660;
mod isobmff;
mod linear;
mod midi;
mod mpeg_ps;
mod mpeg_ts;
mod ogg;
mod ole2;
mod pdf;
mod pe;
mod png;
mod rar;
mod seven_zip;
mod sqlite;
mod tar;
mod text_bound;
mod tiff;
mod ttf;

use ferrite_blockdev::BlockDevice;

use crate::signature::SizeHint;

/// Read the embedded size hint from the device and return the implied total
/// file size.  Returns `None` if any read fails or the header is malformed.
///
/// `max_size` caps the scan window for stream-walking hints like `MpegTs`
/// (they will not read further than `file_offset + max_size` bytes).
/// Other hint variants ignore this parameter.
pub(crate) fn read_size_hint(
    device: &dyn BlockDevice,
    file_offset: u64,
    hint: &SizeHint,
    max_size: u64,
) -> Option<u64> {
    match hint {
        SizeHint::Linear {
            offset,
            len,
            little_endian,
            add,
        } => linear::linear_hint(device, file_offset, *offset, *len, *little_endian, *add),

        SizeHint::Ole2 => ole2::ole2_hint(device, file_offset),

        SizeHint::LinearScaled {
            offset,
            len,
            little_endian,
            scale,
            add,
        } => linear::linear_scaled_hint(
            device,
            file_offset,
            *offset,
            *len,
            *little_endian,
            *scale,
            *add,
        ),

        SizeHint::Sqlite => sqlite::sqlite_hint(device, file_offset),

        SizeHint::SevenZip => seven_zip::seven_zip_hint(device, file_offset),

        SizeHint::OggStream => ogg::ogg_stream_hint(device, file_offset),

        SizeHint::Tiff => tiff::tiff_size_hint(device, file_offset),

        SizeHint::Raf => tiff::raf_size_hint(device, file_offset),

        SizeHint::MpegTs { ts_offset, stride } => {
            mpeg_ts::mpeg_ts_size_hint(device, file_offset, *ts_offset, *stride, max_size)
        }

        SizeHint::Isobmff => isobmff::isobmff_hint(device, file_offset),

        SizeHint::Pe => pe::pe_hint(device, file_offset),

        SizeHint::Elf => elf::elf_hint(device, file_offset),

        SizeHint::Rar => rar::rar_hint(device, file_offset),

        SizeHint::Ebml => ebml::ebml_hint(device, file_offset),

        SizeHint::TextBound => text_bound::text_bound_hint(device, file_offset, max_size),

        SizeHint::Ttf => ttf::ttf_hint(device, file_offset),

        SizeHint::Pdf => pdf::pdf_hint(device, file_offset),

        SizeHint::Gif => gif::gif_hint(device, file_offset, max_size),

        SizeHint::Png => png::png_hint(device, file_offset),

        SizeHint::Iso9660 => iso9660::iso9660_hint(device, file_offset),

        SizeHint::Asf => asf::asf_hint(device, file_offset),

        SizeHint::Tar => tar::tar_hint(device, file_offset, max_size),

        SizeHint::MpegPs => mpeg_ps::mpeg_ps_hint(device, file_offset, max_size),

        SizeHint::Au => au::au_hint(device, file_offset),

        SizeHint::Midi => midi::midi_hint(device, file_offset),
    }
}

#[cfg(test)]
mod tests;
