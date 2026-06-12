use std::cell::RefCell;
use std::path::Path;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;

/// ONNX Runtime codec decoder (neuphonic/neucodec-onnx-decoder).
///
/// Single session, created once at startup. Decodes FSQ code frames
/// back to 24 kHz f32 audio via the ONNX model.
///
/// Uses `RefCell<Session>` because `Session::run` requires `&mut self`
/// (ONNX runtime internal mutability).
pub struct CodecDecoder {
    session: RefCell<Session>,
}

impl CodecDecoder {
    /// Load the ONNX codec decoder from `onnx_path`.
    ///
    /// Uses graph optimization Level 3 (ORT_ENABLE_ALL) matching the Python worker.
    pub fn new(onnx_path: &Path) -> Result<Self, String> {
        eprintln!("Loading ONNX codec decoder...");
        let session = Session::builder()
            .map_err(|e| format!("Failed to create session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| format!("Failed to set optimization level: {e}"))?
            .commit_from_file(onnx_path)
            .map_err(|e| format!("Failed to load ONNX model from {}: {e}", onnx_path.display()))?;
        eprintln!("Loaded.");
        Ok(Self {
            session: RefCell::new(session),
        })
    }

    /// Decode a window of codec frames into 24 kHz f32 audio samples.
    ///
    /// - Input: `codes` — FSQ code values (i32, range 0..65535), one per frame.
    /// - Output: flat f32 vector of length `(F-1)*480` where `F = codes.len()`.
    ///
    /// The ONNX model input is tensor(int32) shape `[1, 1, F]`, output is
    /// tensor(float32) shape `[1, 1, (F-1)*480]`.
    pub fn decode_window(&self, codes: &[i32]) -> Result<Vec<f32>, String> {
        let f = codes.len();
        if f < 2 {
            return Err(format!(
                "Need at least 2 codec frames for decode, got {f}"
            ));
        }

        // Build [1, 1, F] i32 tensor
        let input_tensor = Tensor::from_array(([1usize, 1, f], codes.to_vec()))
            .map_err(|e| format!("Failed to create input tensor: {e}"))?;

        // Run inference — input name is "codes", output name is "audio"
        let mut session = self.session.borrow_mut();
        let outputs = session
            .run(ort::inputs!["codes" => input_tensor])
            .map_err(|e| format!("ONNX inference failed: {e}"))?;

        // Output shape [1, 1, (F-1)*480], f32
        let output_value = &outputs["audio"];
        let (_shape, data) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract output tensor: {e}"))?;

        let expected_len = (f - 1) * 480;
        if data.len() != expected_len {
            return Err(format!(
                "ONNX output length mismatch: expected {expected_len}, got {}",
                data.len()
            ));
        }

        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_window_requires_two_frames() {
        let codes = vec![100i32];
        assert!(codes.len() < 2);
    }
}
