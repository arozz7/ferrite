/// Subprocess execution of `smartctl --json -a`.
use std::process::Command;

use tracing::{debug, warn};
use which::which;

use crate::error::{Result, SmartError};
use crate::parser;
use crate::thresholds::SmartThresholds;
use crate::types::{HealthVerdict, SmartData};
use crate::verdict;

/// Run `smartctl --json -a <device_path>` and return parsed drive data.
///
/// `smartctl_bin` overrides the binary path; when `None` it is located via `$PATH`.
pub fn query(device_path: &str, smartctl_bin: Option<&str>) -> Result<SmartData> {
    let bin = resolve_smartctl(smartctl_bin)?;

    debug!(bin = %bin.display(), device = device_path, "running smartctl");

    let output = Command::new(&bin)
        .args(["--json", "-a", device_path])
        .output()
        .map_err(SmartError::Io)?;

    let exit_code = output.status.code().unwrap_or(-1);

    // Bits 0-1 of the smartctl exit code indicate that the command could not
    // execute or the device could not be opened — no JSON output was produced.
    if exit_code & 0x03 != 0 {
        warn!(code = exit_code, "smartctl hard failure");
        return Err(SmartError::SmartctlError { code: exit_code });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.trim().is_empty() {
        return Err(SmartError::NotSupported);
    }

    let data = parser::parse(device_path, &stdout)?;

    // Exit code bit 2 or 3 means some ATA commands failed / SMART is "FAILING".
    // We have JSON output, but log a warning so the caller is informed.
    if exit_code & 0x0C != 0 {
        warn!(
            code = exit_code,
            device = device_path,
            "smartctl reported drive issues"
        );
    }

    Ok(data)
}

/// Convenience wrapper that also runs the health verdict.
pub fn query_and_assess(
    device_path: &str,
    smartctl_bin: Option<&str>,
    thresholds: &SmartThresholds,
) -> Result<(SmartData, HealthVerdict)> {
    let data = query(device_path, smartctl_bin)?;
    let verdict = verdict::assess_health(&data, thresholds);
    Ok((data, verdict))
}

fn resolve_smartctl(override_bin: Option<&str>) -> Result<std::path::PathBuf> {
    if let Some(bin) = override_bin {
        return Ok(std::path::PathBuf::from(bin));
    }
    which("smartctl").map_err(|_| SmartError::SmartctlNotFound)
}
