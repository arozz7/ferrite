use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 1 — forward sequential copy.
///
/// Reads the device in large blocks (`config.pass_block_sizes[0]`). On
/// success, writes to the output image and marks the range `Finished`. On read
/// error, marks the range `NonTrimmed` and advances (the trim pass isolates
/// exact bad sectors later).
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let device_size = engine.device.size();
    // Clamp so the buffer is never larger than the device and always >= 1 sector.
    let block_size = engine.config.pass_block_sizes[0]
        .min(device_size)
        .max(sector_size);

    let mut buf = AlignedBuffer::new(block_size as usize, sector_size as usize);
    // Second buffer used when verify_reads is enabled — allocate once, reuse.
    let mut verify_buf = if engine.config.verify_reads {
        Some(AlignedBuffer::new(
            block_size as usize,
            sector_size as usize,
        ))
    } else {
        None
    };

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

                        // Optional read-verify: re-read and compare.
                        let verified = verify_read(engine, &mut verify_buf, pos, &buf, to_write);
                        if verified {
                            engine.write_block(pos, &buf.as_slice()[..to_write])?;
                            engine.mapfile.update_range(
                                pos,
                                to_write as u64,
                                BlockStatus::Finished,
                            );
                            debug!(
                                offset = pos,
                                bytes = to_write,
                                "copy: chunk written (reverse)"
                            );
                        } else {
                            engine
                                .mapfile
                                .update_range(pos, chunk_size, BlockStatus::NonTrimmed);
                            warn!(
                                offset = pos,
                                "copy: verify mismatch, marked NonTrimmed (reverse)"
                            );
                        }
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

                        // Optional read-verify: re-read and compare.
                        let verified = verify_read(engine, &mut verify_buf, pos, &buf, to_write);
                        if verified {
                            engine.write_block(pos, &buf.as_slice()[..to_write])?;
                            engine.mapfile.update_range(
                                pos,
                                to_write as u64,
                                BlockStatus::Finished,
                            );
                            debug!(offset = pos, bytes = to_write, "copy: chunk written");
                        } else {
                            engine
                                .mapfile
                                .update_range(pos, chunk_size, BlockStatus::NonTrimmed);
                            warn!(offset = pos, "copy: verify mismatch, marked NonTrimmed");
                        }
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Re-read `len` bytes at `offset` into `vbuf` and compare with `original`.
///
/// Returns `true` (verified) when `verify_reads` is disabled, or when all
/// `verify_passes` reads match the original data.  Returns `false` on any read
/// failure or data mismatch, signalling the caller to mark the block
/// `NonTrimmed` rather than writing potentially corrupt data.
fn verify_read(
    engine: &mut ImagingEngine,
    verify_buf: &mut Option<AlignedBuffer>,
    offset: u64,
    original: &AlignedBuffer,
    len: usize,
) -> bool {
    let vbuf = match verify_buf {
        Some(b) => b,
        None => return true, // verify_reads disabled
    };
    let passes = engine.config.verify_passes.max(1) as usize;
    for _ in 0..passes {
        match engine.device.read_at(offset, vbuf) {
            Ok(vn) if vn >= len => {
                if vbuf.as_slice()[..len] != original.as_slice()[..len] {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}
