use std::io::{Seek, SeekFrom, Write};

use ferrite_blockdev::AlignedBuffer;
use tracing::debug;

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 3 — sweep: handle any `NonTried` blocks that remain after the copy pass.
///
/// Reads sector-by-sector (no large-block skipping). On the first failure,
/// marks that sector `BadSector` and the remainder `NonScraped`, then stops
/// processing that block — identical to the trim pass strategy.
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let mut buf = AlignedBuffer::new(sector_size as usize, sector_size as usize);

    let work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::NonTried)
        .collect();

    for region in work {
        let mut pos = region.pos;

        while pos < region.end() {
            let chunk = (region.end() - pos).min(sector_size);

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

                    debug!(offset = pos, "sweep: sector ok");
                    pos += to_write as u64;
                }
                Err(_) => {
                    engine
                        .mapfile
                        .update_range(pos, chunk, BlockStatus::BadSector);
                    let rest_start = pos + chunk;
                    let rest_size = region.end().saturating_sub(rest_start);
                    if rest_size > 0 {
                        engine
                            .mapfile
                            .update_range(rest_start, rest_size, BlockStatus::NonScraped);
                    }
                    debug!(
                        offset = pos,
                        "sweep: first failure — rest marked NonScraped"
                    );
                    break;
                }
            }

            engine.maybe_save_mapfile()?;
            let progress = engine.make_progress(ImagingPhase::Sweep, pos);
            if reporter.report(&progress) == Signal::Cancel {
                return Err(ImagingError::Cancelled);
            }
        }
    }

    Ok(())
}
