use std::io::{Seek, SeekFrom, Write};

use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 1 — forward sequential copy.
///
/// Reads the device in large blocks (`config.copy_block_size`). On success,
/// writes to the output image and marks the range `Finished`. On read error,
/// marks the range `NonTrimmed` and advances (the trim pass isolates exact bad
/// sectors later).
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let device_size = engine.device.size();
    // Clamp so the buffer is never larger than the device and always >= 1 sector.
    let block_size = engine
        .config
        .copy_block_size
        .min(device_size)
        .max(sector_size);

    let mut buf = AlignedBuffer::new(block_size as usize, sector_size as usize);

    // Snapshot before iterating — update_range mutably borrows the mapfile.
    let mut work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::NonTried)
        .collect();

    if engine.config.reverse {
        work.reverse();
    }

    'region: for region in work {
        if engine.config.reverse {
            // Collect all chunk start positions then iterate end→start.
            let mut positions: Vec<(u64, u64)> = Vec::new();
            let mut p = region.pos;
            while p < region.end() {
                let remaining = region.end() - p;
                let chunk = remaining.min(block_size);
                positions.push((p, chunk));
                p += chunk;
            }
            positions.reverse();

            for (pos, chunk_size) in positions {
                match engine.device.read_at(pos, &mut buf) {
                    Ok(0) => {
                        break 'region;
                    }
                    Ok(n) => {
                        let to_write = n.min(chunk_size as usize);

                        engine
                            .output
                            .seek(SeekFrom::Start(pos))
                            .and_then(|_| engine.output.write_all(&buf.as_slice()[..to_write]))
                            .map_err(|e| ImagingError::ImageWrite {
                                offset: pos,
                                source: e,
                            })?;

                        engine
                            .mapfile
                            .update_range(pos, to_write as u64, BlockStatus::Finished);

                        debug!(
                            offset = pos,
                            bytes = to_write,
                            "copy: chunk written (reverse)"
                        );
                    }
                    Err(_) => {
                        engine
                            .mapfile
                            .update_range(pos, chunk_size, BlockStatus::NonTrimmed);

                        warn!(
                            offset = pos,
                            "copy: read error, marked NonTrimmed (reverse)"
                        );
                    }
                }

                engine.maybe_save_mapfile()?;
                let progress = engine.make_progress(ImagingPhase::Copy, pos);
                if reporter.report(&progress) == Signal::Cancel {
                    return Err(ImagingError::Cancelled);
                }
            }
        } else {
            let mut pos = region.pos;

            while pos < region.end() {
                let remaining = region.end() - pos;
                let chunk_size = remaining.min(block_size);

                match engine.device.read_at(pos, &mut buf) {
                    Ok(0) => {
                        // Past end-of-device.
                        break 'region;
                    }
                    Ok(n) => {
                        // Use at most `chunk_size` bytes (buf may contain bytes past
                        // the region end if chunk_size < block_size).
                        let to_write = n.min(chunk_size as usize);

                        engine
                            .output
                            .seek(SeekFrom::Start(pos))
                            .and_then(|_| engine.output.write_all(&buf.as_slice()[..to_write]))
                            .map_err(|e| ImagingError::ImageWrite {
                                offset: pos,
                                source: e,
                            })?;

                        engine
                            .mapfile
                            .update_range(pos, to_write as u64, BlockStatus::Finished);

                        debug!(offset = pos, bytes = to_write, "copy: chunk written");
                        pos += to_write as u64;
                    }
                    Err(_) => {
                        // Record the failure and skip — do not propagate the error.
                        engine
                            .mapfile
                            .update_range(pos, chunk_size, BlockStatus::NonTrimmed);

                        warn!(offset = pos, "copy: read error, marked NonTrimmed");
                        pos += chunk_size;
                    }
                }

                engine.maybe_save_mapfile()?;
                let progress = engine.make_progress(ImagingPhase::Copy, pos);
                if reporter.report(&progress) == Signal::Cancel {
                    return Err(ImagingError::Cancelled);
                }
            }
        }
    }

    Ok(())
}
