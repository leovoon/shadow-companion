//! stdin/stdout protocol for the TTS worker subprocess.
//!
//! See RUST_WORKER_PLAN §6 for the parity checklist.

use std::io::{self, BufRead, Write};

/// The exact line printed (and flushed) once models are loaded and prewarmed.
pub const WORKER_READY: &str = "WORKER_READY";

/// Maximum input text length in Unicode code points before truncation.
const MAX_INPUT_CHARS: usize = 500;

/// Read one non-empty trimmed line from stdin. Returns `None` on EOF.
pub fn read_stdin_line() -> Option<String> {
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    loop {
        let mut line = String::new();
        match handle.read_line(&mut line) {
            Ok(0) => return None,        // EOF
            Ok(_) => {
                let trimmed = line.trim().to_owned();
                if trimmed.is_empty() {
                    continue;            // skip blank lines
                }
                return Some(trimmed);
            }
            Err(_) => return None,       // EINTR / broken pipe → treat as EOF
        }
    }
}

/// Parse a raw stdin line into input text.
///
/// Primary: try JSON `{"text": "..."}`, extract the `"text"` value trimmed.
/// Fallback: invalid JSON → the raw line is the text.
/// Truncate to 500 Unicode chars + "..." if longer.
pub fn parse_text_input(line: &str) -> String {
    let raw = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(map)) => {
            match map.get("text") {
                Some(serde_json::Value::String(s)) => s.trim().to_owned(),
                _ => String::new(), // missing or non-string "text" → empty → skip
            }
        }
        _ => line.to_owned(), // not a JSON object → fallback to raw line
    };
    truncate_input(&raw)
}

/// Truncate to 500 Unicode code points, appending "..." if truncated.
fn truncate_input(text: &str) -> String {
    if text.chars().count() <= MAX_INPUT_CHARS {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(MAX_INPUT_CHARS).collect();
        format!("{truncated}...")
    }
}

/// Print `WORKER_READY` and flush stdout immediately.
pub fn print_worker_ready() {
    println!("{WORKER_READY}");
    let _ = io::stdout().flush();
}

/// Print `"{duration_s:.1}s played"` and flush stdout.
pub fn print_played(duration_s: f64) {
    println!("{duration_s:.1}s played");
    let _ = io::stdout().flush();
}

/// Print `"TTS error: {msg}"` and flush stdout.
pub fn print_error(msg: &str) {
    println!("TTS error: {msg}");
    let _ = io::stdout().flush();
}

/// Print a free-form log line and flush stdout.
pub fn print_log(msg: &str) {
    println!("{msg}");
    let _ = io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_text() {
        let line = r#"{"text": "Hello, world!"}"#;
        assert_eq!(parse_text_input(line), "Hello, world!");
    }

    #[test]
    fn parse_json_text_trimmed() {
        let line = r#"{"text": "  padded  "}"#;
        assert_eq!(parse_text_input(line), "padded");
    }

    #[test]
    fn parse_json_missing_text() {
        let line = r#"{"other": "value"}"#;
        assert_eq!(parse_text_input(line), "");
    }

    #[test]
    fn parse_json_non_string_text() {
        let line = r#"{"text": 42}"#;
        assert_eq!(parse_text_input(line), "");
    }

    #[test]
    fn parse_fallback_raw_line() {
        let line = "Just plain text";
        assert_eq!(parse_text_input(line), "Just plain text");
    }

    #[test]
    fn parse_fallback_invalid_json() {
        let line = "not json at all";
        assert_eq!(parse_text_input(line), "not json at all");
    }

    #[test]
    fn truncate_under_limit() {
        let text = "short".repeat(10); // 50 chars
        assert_eq!(truncate_input(&text), text);
        assert!(!truncate_input(&text).ends_with("..."));
    }

    #[test]
    fn truncate_at_limit() {
        let text: String = "a".repeat(500);
        assert_eq!(truncate_input(&text), text);
    }

    #[test]
    fn truncate_over_limit() {
        let text: String = "a".repeat(501);
        let result = truncate_input(&text);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 503); // 500 + 3 for "..."
    }

    #[test]
    fn truncate_unicode_chars() {
        // "é" is one code point but two bytes in UTF-8 — count by chars, not bytes
        let text: String = "é".repeat(501);
        let result = truncate_input(&text);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 503);
    }

    #[test]
    fn truncate_multibyte_unicode() {
        // Emoji: 1 code point each, multiple UTF-8 bytes
        let text: String = "🎉".repeat(501);
        let result = truncate_input(&text);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 503);
    }
}
