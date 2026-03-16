use std::io::{Seek, SeekFrom, Write};

use ferrite_blockdev::AlignedBuffer;
use tracing::debug;

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
    let mut buf = AlignedBuffer::new(sector_size as usize, sector_size as usize);
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
        let mut pos = if attempt.is_multiple_of(2) {
            region.pos
        } else {
            region.end().saturating_sub(sector_size)
        };

        loop {
            let chunk = (region.end() - region.pos).min(sector_size);

            match engine.device.read_at(pos, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let to_write = n.min(chunk as usize);
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

                    debug!(offset = pos, attempt, "retry: sector recovered");
                }
                Err(_) => {
                    debug!(offset = pos, attempt, "retry: sector still bad");
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
