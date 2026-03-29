use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 2 — trim: isolate exact bad-sector locations within NonTrimmed blocks.
///
/// For each `NonTrimmed` block, reads sector-by-sector from the leading edge.
/// The first failure marks that sector `BadSector` and all remaining bytes in
/// the block `NonScraped` (to be attempted by the scrape pass). Sectors before
/// the failure are marked `Finished`.
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let mut buf = AlignedBuffer::new(sector_size as usize, sector_size as usize);

    let work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::NonTrimmed)
        .collect();

    for region in work {
        let mut pos = region.pos;

        while pos < region.end() {
            let chunk = (region.end() - pos).min(sector_size);

            match engine.device.read_at(pos, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let to_write = n.min(chunk as usize);
                    engine.write_block(pos, &buf.as_slice()[..to_write])?;

                    engine
                        .mapfile
                        .update_range(pos, to_write as u64, BlockStatus::Finished);

                    debug!(offset = pos, "trim: sector ok");
                    pos += to_write as u64;
                }
                Err(_) => {
                    // Mark this one sector as BadSector.
                    engine
                        .mapfile
                        .update_range(pos, chunk, BlockStatus::BadSector);
                    // Mark everything remaining in the block as NonScraped.
                    let rest_start = pos + chunk;
                    let rest_size = region.end().saturating_sub(rest_start);
                    if rest_size > 0 {
                        engine
                            .mapfile
                            .update_range(rest_start, rest_size, BlockStatus::NonScraped);
                    }
                    warn!(offset = pos, "trim: first failure — rest marked NonScraped");
                    break; // Stop trimming this block.
                }
            }

            engine.maybe_save_mapfile()?;
            let progress = engine.make_progress(ImagingPhase::Trim, pos);
            if reporter.report(&progress) == Signal::Cancel {
                return Err(ImagingError::Cancelled);
            }
        }
    }

    Ok(())
}
