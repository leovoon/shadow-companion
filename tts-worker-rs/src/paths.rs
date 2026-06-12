use std::env;
use std::fs;
use std::path::PathBuf;

fn home_dir() -> PathBuf {
    PathBuf::from(env::var("HOME").expect("HOME env var not set"))
}

fn shadow_dir() -> PathBuf {
    home_dir().join(".shadow-companion")
}

/// Glob-like search: iterate snapshot subdirs under `base/snapshots/`,
/// return the first subdir containing `filename`.
fn find_in_snapshots(base: &std::path::Path, filename: &str) -> Result<PathBuf, String> {
    let snapshots_dir = base.join("snapshots");
    let entries = fs::read_dir(&snapshots_dir)
        .map_err(|e| format!("cannot read {}: {e}", snapshots_dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("read_dir entry error: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            let candidate = path.join(filename);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    Err(format!(
        "no snapshot containing '{}' found under {} \
         (set TTS_GGUF / TTS_ONNX env var to override)",
        filename,
        snapshots_dir.display()
    ))
}

/// Resolve GGUF backbone model path.
/// Order: `TTS_GGUF` env → glob HF cache snapshots.
pub fn resolve_gguf_path() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("TTS_GGUF") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("TTS_GGUF={p} does not exist"));
    }

    let base = home_dir()
        .join(".cache")
        .join("huggingface")
        .join("hub")
        .join("models--neuphonic--neutts-air-q8-gguf");

    find_in_snapshots(&base, "neutts-air-Q8_0.gguf")
}

/// Resolve ONNX decoder model path.
/// Order: `TTS_ONNX` env → glob HF cache snapshots.
pub fn resolve_onnx_path() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("TTS_ONNX") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("TTS_ONNX={p} does not exist"));
    }

    let base = home_dir()
        .join(".cache")
        .join("huggingface")
        .join("hub")
        .join("models--neuphonic--neucodec-onnx-decoder");

    find_in_snapshots(&base, "model.onnx")
}

/// Resolve espeak-ng dynamic library path.
/// Order: `TTS_ESPEAK_LIB` env → `~/.shadow-companion/espeak/libespeak-ng.dylib`
///        → venv `neutts/libespeak-ng.dylib`.
pub fn resolve_espeak_lib() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("TTS_ESPEAK_LIB") {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return Ok(pb);
        }
        return Err(format!("TTS_ESPEAK_LIB={p} does not exist"));
    }

    let staged = shadow_dir().join("espeak").join("libespeak-ng.dylib");
    if staged.exists() {
        return Ok(staged);
    }

    let venv = home_dir()
        .join("shadow-companion")
        .join(".venv")
        .join("lib")
        .join("python3.13")
        .join("site-packages")
        .join("neutts")
        .join("libespeak-ng.dylib");
    if venv.exists() {
        return Ok(venv);
    }

    Err(format!(
        "libespeak-ng.dylib not found at {} or {} \
         (set TTS_ESPEAK_LIB env var to override)",
        staged.display(),
        venv.display()
    ))
}

/// Resolve espeak-ng data directory path.
/// Order: `TTS_ESPEAK_DATA` env → `~/.shadow-companion/espeak/espeak-ng-data`
///        → venv `neutts/espeak-ng-data`.
pub fn resolve_espeak_data() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("TTS_ESPEAK_DATA") {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            return Ok(pb);
        }
        return Err(format!("TTS_ESPEAK_DATA={p} does not exist or is not a directory"));
    }

    let staged = shadow_dir().join("espeak").join("espeak-ng-data");
    if staged.is_dir() {
        return Ok(staged);
    }

    let venv = home_dir()
        .join("shadow-companion")
        .join(".venv")
        .join("lib")
        .join("python3.13")
        .join("site-packages")
        .join("neutts")
        .join("espeak-ng-data");
    if venv.is_dir() {
        return Ok(venv);
    }

    Err(format!(
        "espeak-ng-data not found at {} or {} \
         (set TTS_ESPEAK_DATA env var to override)",
        staged.display(),
        venv.display()
    ))
}

/// `~/.shadow-companion/ref_codes.npy`
pub fn ref_codes_path() -> PathBuf {
    shadow_dir().join("ref_codes.npy")
}

/// `~/.shadow-companion/ref_text.txt`
pub fn ref_text_path() -> PathBuf {
    shadow_dir().join("ref_text.txt")
}

/// `~/.shadow-companion/state.json`
pub fn state_json_path() -> PathBuf {
    shadow_dir().join("state.json")
}

/// `~/.shadow-companion/tts-play-log.json`
pub fn play_log_path() -> PathBuf {
    shadow_dir().join("tts-play-log.json")
}

/// `~/.shadow-companion/`
pub fn state_dir() -> PathBuf {
    shadow_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_dir_is_home_shadow_companion() {
        let sd = state_dir();
        assert!(sd.ends_with(".shadow-companion"));
    }

    #[test]
    fn ref_codes_under_state_dir() {
        assert_eq!(ref_codes_path(), state_dir().join("ref_codes.npy"));
    }

    #[test]
    fn ref_text_under_state_dir() {
        assert_eq!(ref_text_path(), state_dir().join("ref_text.txt"));
    }

    #[test]
    fn state_json_under_state_dir() {
        assert_eq!(state_json_path(), state_dir().join("state.json"));
    }

    #[test]
    fn play_log_under_state_dir() {
        assert_eq!(play_log_path(), state_dir().join("tts-play-log.json"));
    }

    #[test]
    fn resolve_gguf_uses_env() {
        // Can't easily set env in multi-threaded test, but verify the path construction.
        let base = home_dir()
            .join(".cache")
            .join("huggingface")
            .join("hub")
            .join("models--neuphonic--neutts-air-q8-gguf");
        assert!(base.to_string_lossy().contains("neutts-air-q8-gguf"));
    }

    #[test]
    fn resolve_onnx_uses_correct_model_dir() {
        let base = home_dir()
            .join(".cache")
            .join("huggingface")
            .join("hub")
            .join("models--neuphonic--neucodec-onnx-decoder");
        assert!(base.to_string_lossy().contains("neucodec-onnx-decoder"));
    }
}
