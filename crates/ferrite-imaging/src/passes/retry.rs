use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 5 — retry: re-attempt each `BadSector` up to `max_retries` times.
///
/// Direction alternates each attempt (forward on even, reverse on odd) to
/// improve chances against marginal sectors with head-alignment sensitivity.
/// Successes become `Finished`; persistent failures remain `BadSector`.
pub(crate) fn run(
    engine: &mut ImagingEngine,
    reporter: &mut dyn ProgressReporter,
    attempt: u32,
) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let block_size = engine.config.pass_block_sizes[4].max(sector_size);
    let mut buf = AlignedBuffer::new(block_size as usize, sector_size as usize);
    let max = engine.config.max_retries;
    let phase = ImagingPhase::Retry {
        attempt: attempt + 1,
        max,
    };

    // Collect sectors; reverse on odd attempts.
    let mut work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::BadSector)
        .collect();
    if attempt % 2 == 1 {
        work.reverse();
    }

    for region in work {
        // Direction per attempt:
        //   Even (0, 2, …): forward  — pos starts at region.pos, advances by chunk,
        //                              exits when pos >= region.end()
        //   Odd  (1, 3, …): reverse  — pos starts at region.end() - block_size,
        //                              retreats by chunk, exits when stepping back
        //                              would go below region.pos (pos < region.pos + chunk)
        //
        // `chunk` is clamped to the region size so partial-block regions at the
        // end of the device are handled correctly.
        let mut pos = if attempt.is_multiple_of(2) {
            region.pos
        } else {
            region.end().saturating_sub(block_size)
        };

        loop {
            let chunk = (region.end() - region.pos).min(block_size);

            match engine.device.read_at(pos, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let to_write = n.min(chunk as usize);
                    engine.write_block(pos, &buf.as_slice()[..to_write])?;

                    engine
                        .mapfile
                        .update_range(pos, to_write as u64, BlockStatus::Finished);

                    debug!(offset = pos, attempt, "retry: sector recovered");
                }
                Err(_) => {
                    warn!(offset = pos, attempt, "retry: sector still bad");
                    // Stays BadSector — no status change needed.
                }
            }

            engine.maybe_save_mapfile()?;
            let progress = engine.make_progress(phase, pos);
            if reporter.report(&progress) == Signal::Cancel {
                return Err(ImagingError::Cancelled);
            }

            // Advance in the correct direction.
            if attempt.is_multiple_of(2) {
                pos += chunk;
                if pos >= region.end() {
                    break;
                }
            } else {
                if pos < region.pos + chunk {
                    break;
                }
                pos -= chunk;
            }
        }
    }

    Ok(())
}
