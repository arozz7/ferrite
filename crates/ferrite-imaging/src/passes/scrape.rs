use std::io::{Seek, SeekFrom, Write};

use ferrite_blockdev::AlignedBuffer;
use tracing::{debug, warn};

use crate::engine::ImagingEngine;
use crate::error::{ImagingError, Result};
use crate::mapfile::BlockStatus;
use crate::progress::{ImagingPhase, Signal};
use crate::ProgressReporter;

/// Pass 4 — scrape: attempt every `NonScraped` sector individually.
///
/// Unlike the trim and sweep passes, scrape does **not** stop on the first
/// failure — every sector is tried independently. Successes become `Finished`;
/// failures become `BadSector` (to be retried in pass 5).
pub(crate) fn run(engine: &mut ImagingEngine, reporter: &mut dyn ProgressReporter) -> Result<()> {
    let sector_size = engine.device.sector_size() as u64;
    let mut buf = AlignedBuffer::new(sector_size as usize, sector_size as usize);

    let work: Vec<_> = engine
        .mapfile
        .blocks_with_status(BlockStatus::NonScraped)
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

                    debug!(offset = pos, "scrape: sector ok");
                }
                Err(_) => {
                    engine
                        .mapfile
                        .update_range(pos, chunk, BlockStatus::BadSector);
                    warn!(offset = pos, "scrape: sector failed → BadSector");
                }
            }

            // Always advance — scrape tries every sector.
            pos += chunk;

            engine.maybe_save_mapfile()?;
            let progress = engine.make_progress(ImagingPhase::Scrape, pos);
            if reporter.report(&progress) == Signal::Cancel {
                return Err(ImagingError::Cancelled);
            }
        }
    }

    Ok(())
}
