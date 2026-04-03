use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 3 — sweep: handle any `NonTried` blocks that remain after the copy pass.
///
/// Reads in chunks of `config.pass_block_sizes[2]` (clamped to `sector_size`
/// minimum). On the first failure, marks that chunk `BadSector` and the
/// remainder `NonScraped`, then stops processing that block — identical to the
/// trim pass strategy.
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let block_size = engine.config.pass_block_sizes[2].max(sector_size);
    let mut buf = AlignedBuffer::new(block_size as usize, sector_size as usize);

    let work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::NonTried)
        .collect();

    for region in work {
        let mut pos = region.pos;

        while pos < region.end() {
            let chunk = (region.end() - pos).min(block_size);

            match engine.device.read_at(pos, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let to_write = n.min(chunk as usize);
                    engine.write_block(pos, &buf.as_slice()[..to_write])?;

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
                    warn!(
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
