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

    let translated = translate_device_path(device_path);
    debug!(bin = %bin.display(), device = device_path, translated = %translated, "running smartctl");

    let output = Command::new(&bin)
        .args(["--json", "-a", &translated])
        .output()
        .map_err(SmartError::Io)?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Always attempt JSON parsing first — on Windows, smartctl frequently exits
    // with code 1 (permission/NVMe quirks) but still emits valid JSON output.
    // Only treat bits 0-1 as a hard failure when stdout is empty or unparseable.
    if exit_code & 0x03 != 0 && stdout.trim().is_empty() {
        warn!(code = exit_code, "smartctl hard failure, no output");
        return Err(SmartError::SmartctlError { code: exit_code });
    }

    if stdout.trim().is_empty() {
        return Err(SmartError::NotSupported);
    }

    let data = parser::parse(device_path, &stdout).map_err(|e| {
        // If bits 0-1 were set and parsing also failed, surface the exit code error
        // as it is more actionable (e.g. "run as Administrator").
        if exit_code & 0x03 != 0 {
            warn!(code = exit_code, "smartctl hard failure, JSON unparseable");
            SmartError::SmartctlError { code: exit_code }
        } else {
            e
        }
    })?;

    // Bits 2-3: ATA commands failed or SMART is FAILING — data was returned but
    // the drive is reporting problems. Log and continue; verdict will reflect it.
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

/// Translate a platform device path to the format smartctl expects.
///
/// On Windows the AppVeyor/MinGW build of smartctl does not accept
/// `\\.\PhysicalDriveN` — it requires `/dev/sda`-style paths instead.
/// Index 0 → `/dev/sda`, 1 → `/dev/sdb`, …, 25 → `/dev/sdz`, 26 → `/dev/sdaa`, etc.
fn translate_device_path(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let prefix = r"\\.\PhysicalDrive";
        if let Some(n_str) = path.strip_prefix(prefix) {
            if let Ok(n) = n_str.parse::<usize>() {
                return format!("/dev/sd{}", index_to_drive_letters(n));
            }
        }
    }
    path.to_string()
}

/// Convert a zero-based drive index to a letter suffix: 0→"a", 25→"z", 26→"aa", …
fn index_to_drive_letters(mut n: usize) -> String {
    let mut letters = String::new();
    loop {
        letters.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    letters
}
