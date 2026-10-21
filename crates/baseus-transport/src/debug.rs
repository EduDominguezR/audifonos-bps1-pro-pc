//! Optional raw-traffic log for field reports.
//!
//! Disabled unless the app is started with `--debug` (or `BASEUS_DEBUG=1`).
//! Writes every notification and every write, in hex, to
//! `%APPDATA%\AudifonosBaseus\debug.log`, rotating at 512 KB.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 512 * 1024;

static ENABLED: AtomicBool = AtomicBool::new(false);

struct Log {
    file: Option<File>,
    path: PathBuf,
    bytes: u64,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// One line per direction: `TX` for writes, `RX` for notifications.
pub fn traffic(direction: &str, bytes: &[u8]) {
    if !is_enabled() {
        return;
    }
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    write_line(&format!("{direction} {}", hex.join(" ")));
}

/// Notes that do not fit a direction, such as an undocumented battery byte.
pub fn note(text: &str) {
    if !is_enabled() {
        return;
    }
    write_line(&format!("nota: {text}"));
}

fn write_line(text: &str) {
    let line = format!("[{}] {text}\n", timestamp());
    let log = LOG.get_or_init(|| {
        Mutex::new(Log {
            file: None,
            path: path(),
            bytes: 0,
        })
    });
    let Ok(mut log) = log.lock() else {
        return;
    };
    if log.file.is_none() {
        open(&mut log);
    }
    if log.bytes + line.len() as u64 > MAX_BYTES {
        rotate(&mut log);
    }
    if let Some(file) = log.file.as_mut() {
        if file.write_all(line.as_bytes()).is_ok() {
            log.bytes += line.len() as u64;
        }
    }
}

fn open(log: &mut Log) {
    if let Some(parent) = log.path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    log.bytes = std::fs::metadata(&log.path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if log.bytes > MAX_BYTES {
        rotate(log);
        return;
    }
    log.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log.path)
        .ok();
}

fn rotate(log: &mut Log) {
    log.file = None;
    let old = log.path.with_extension("log.1");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(&log.path, &old);
    log.bytes = 0;
    log.file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log.path)
        .ok();
}

fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AudifonosBaseus").join("debug.log")
}

/// UTC timestamp with milliseconds, computed without extra dependencies.
fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{millis:03}",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm, valid for any proleptic date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_415), (2025, 11, 23));
    }
}
