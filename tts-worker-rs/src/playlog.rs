//! tts-play-log.json read-modify-write (plan §5.13, shadow.py:100-114).
//!
//! Flat JSON map `local-ISO-date → cumulative seconds`, 2-space indent.
//! Corrupt/unparsable file silently resets to `{}`.
//! Historical keys preserved (BTreeMap alphabetical order).

use std::collections::BTreeMap;
use std::fs;

use chrono::Local;
use serde_json;

use crate::paths;

/// Append `duration_s` to today's cumulative entry in `tts-play-log.json`.
///
/// Mirrors Python `log_tts_play` (shadow.py:100-114):
///   1. `mkdir ~/.shadow-companion/`
///   2. Read file → if exists + parseable JSON map, use; else `{}`
///   3. Today local date ISO `YYYY-MM-DD`
///   4. `log[today] = round(prev + duration, 1)`
///   5. Write with `serde_json::to_string_pretty` (2-space indent)
pub fn log_play(duration_s: f64) {
    let dir = paths::state_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("playlog: mkdir {}: {e}", dir.display());
        return;
    }

    let path = paths::play_log_path();

    // Read existing log; corrupt/unparsable → empty map
    let mut log: BTreeMap<String, f64> = if path.exists() {
        match fs::read_to_string(&path) {
            Ok(txt) => serde_json::from_str(&txt).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        }
    } else {
        BTreeMap::new()
    };

    // Today's key
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Accumulate and round to 1 decimal
    let prev = log.get(&today).copied().unwrap_or(0.0);
    log.insert(today, round1(prev + duration_s));

    // Write back (2-space indent via to_string_pretty default)
    let json = serde_json::to_string_pretty(&log).unwrap_or_default();
    if let Err(e) = fs::write(&path, json) {
        eprintln!("playlog: write {}: {e}", path.display());
    }
}

/// Round to 1 decimal place, matching Python `round(x, 1)`.
/// Uses `f64::round` after scaling: `round(x * 10.0) / 10.0`.
fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round1_basic() {
        assert_eq!(round1(0.0), 0.0);
        assert_eq!(round1(66.05), 66.0);
        assert_eq!(round1(271.46), 271.5);
        assert_eq!(round1(0.15), 0.2);
        assert_eq!(round1(0.25), 0.3); // banker's rounding differs, but f64::round rounds half-up
    }

    #[test]
    fn btreemap_key_order() {
        let mut m: BTreeMap<String, f64> = BTreeMap::new();
        m.insert("2026-06-11".into(), 271.5);
        m.insert("2026-06-10".into(), 66.0);
        let json = serde_json::to_string_pretty(&m).unwrap();
        // "2026-06-10" must appear before "2026-06-11"
        let pos10 = json.find("2026-06-10").unwrap();
        let pos11 = json.find("2026-06-11").unwrap();
        assert!(pos10 < pos11, "BTreeMap keys must be sorted alphabetically");
    }

    #[test]
    fn serde_f64_renders_dot_zero() {
        let mut m: BTreeMap<String, f64> = BTreeMap::new();
        m.insert("2026-06-10".into(), 66.0);
        let json = serde_json::to_string_pretty(&m).unwrap();
        // serde_json renders f64 66.0 as "66.0" (not "66")
        assert!(json.contains("66.0"), "f64 values must render with decimal: {json}");
    }
}
