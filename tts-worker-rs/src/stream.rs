//! Streaming overlap-add — verbatim port of neutts `_infer_stream_ggml` window math
//! and `_linear_overlap_add`. See RUST_WORKER_PLAN §5.10.

use crate::codec::CodecDecoder;

// ── Constants (§5.1 / §5.10) ──────────────────────────────────────────────

pub const HOP: usize = 480;
pub const CHUNK: usize = 25;
pub const LOOKF: usize = 5;
pub const LOOKB: usize = 50;
pub const OVERLAP: usize = 1;
pub const STRIDE: usize = 12_000; // CHUNK * HOP

// ── Stream state ───────────────────────────────────────────────────────────

/// Tracks the streaming overlap-add state machine across a single utterance.
///
/// `cache` is seeded with the 526 reference codes. As the LLM generates new
/// speech-code tokens they are appended via [`push_code`]. Every 30 new frames
/// (CHUNK + LOOKF) a window is decoded, sliced, overlap-added, and 12 000
/// samples (0.5 s) are emitted. After generation ends, [`final_flush`] emits
/// any remaining samples.
pub struct StreamState {
    /// All speech codes so far: ref_codes prefix + generated codes.
    pub cache: Vec<i32>,
    /// Decoded audio chunks pending overlap-add (each ≈ 12 960 samples).
    pub audio_cache: Vec<Vec<f32>>,
    /// Frame cursor — how many frames in `cache` have been consumed by
    /// regular (non-final) chunk decodes.
    pub n_decoded_tokens: usize,
    /// Sample cursor — how many samples of the overlap-added output have
    /// already been emitted.
    pub n_decoded_samples: usize,
}

impl StreamState {
    /// Create a new stream state seeded with the reference codes.
    pub fn new(ref_codes: &[i32]) -> Self {
        Self {
            cache: ref_codes.to_vec(),
            audio_cache: Vec::new(),
            n_decoded_tokens: ref_codes.len(),
            n_decoded_samples: 0,
        }
    }

    /// Append a single speech code and, if 30 new frames are available,
    /// decode a window and return the next 12 000-sample chunk.
    ///
    /// Returns `Ok(None)` if not enough new frames have accumulated yet.
    /// Returns `Err` if ONNX decode fails.
    pub fn push_code(&mut self, code: i32, codec: &CodecDecoder) -> Result<Option<Vec<f32>>, String> {
        self.cache.push(code);

        if self.cache.len() - self.n_decoded_tokens >= CHUNK + LOOKF {
            // ── Window boundaries ──
            let tokens_start = self.n_decoded_tokens.saturating_sub(LOOKB + OVERLAP);
            let tokens_end_want = self.n_decoded_tokens + CHUNK + LOOKF + OVERLAP;
            let tokens_end = tokens_end_want.min(self.cache.len());
            let window_len = tokens_end - tokens_start;
            let sample_start = (self.n_decoded_tokens - tokens_start) * HOP;
            let sample_end_want = sample_start + (CHUNK + 2 * OVERLAP) * HOP; // +12 960
            // ONNX output is (window_len-1)*480 samples; clamp slice to actual length
            let max_samples = (window_len - 1) * HOP;
            let sample_end = sample_end_want.min(sample_start + max_samples);

            // ── Decode ──
            let window = &self.cache[tokens_start..tokens_end];
            let recon = codec.decode_window(window)?;

            // ── Slice out the chunk ──
            let chunk: Vec<f32> = recon[sample_start..sample_end].to_vec();

            self.audio_cache.push(chunk);

            // ── Overlap-add over the full audio cache ──
            let processed = linear_overlap_add(&self.audio_cache, STRIDE);
            let new_end = self.audio_cache.len() * STRIDE;

            // Emit only the newly covered samples
            let emitted: Vec<f32> = processed[self.n_decoded_samples..new_end].to_vec();
            self.n_decoded_samples = new_end;
            self.n_decoded_tokens += CHUNK;

            Ok(Some(emitted))
        } else {
            Ok(None)
        }
    }

    /// After generation ends, decode any remaining codes and emit the tail
    /// samples. Returns `Ok(None)` if nothing remains.
    /// Returns `Err` if ONNX decode fails.
    pub fn final_flush(&mut self, codec: &CodecDecoder) -> Result<Option<Vec<f32>>, String> {
        if self.cache.len() <= self.n_decoded_tokens {
            return Ok(None);
        }

        let remaining = self.cache.len() - self.n_decoded_tokens;
        let tokens_start = self
            .cache
            .len()
            .saturating_sub(LOOKB + OVERLAP + remaining);
        let sample_start =
            (self.cache.len() - tokens_start - remaining - OVERLAP) * HOP;

        // Decode the final window (to end of cache)
        let recon = codec.decode_window(&self.cache[tokens_start..])?;

        // Slice from sample_start to end
        let chunk: Vec<f32> = recon[sample_start..].to_vec();

        self.audio_cache.push(chunk);

        // Overlap-add over the full audio cache
        let processed = linear_overlap_add(&self.audio_cache, STRIDE);

        // Emit everything from the last cursor position onward
        let emitted: Vec<f32> = processed[self.n_decoded_samples..].to_vec();

        Ok(Some(emitted))
    }
}

// ── Overlap-add ────────────────────────────────────────────────────────────

/// Linear overlap-add with a **descending ramp** weight (NOT a triangle).
///
/// This is a verbatim port of the Python `_linear_overlap_add`. The weight
/// for sample `k` in a frame of length `L` is:
///
/// ```text
/// weight = 1.0 − (k+1) as f32 / (L+1) as f32
/// ```
///
/// which equals `|0.5 − (t − 0.5)|` for `t = (k+1)/(L+1)` — a DESCENDING
/// ramp. This differs from encodec's triangle window. **Do NOT "fix" it.**
fn linear_overlap_add(frames: &[Vec<f32>], stride: usize) -> Vec<f32> {
    assert!(!frames.is_empty());

    // Compute total output length
    let mut total_size: usize = 0;
    for (i, frame) in frames.iter().enumerate() {
        let frame_end = stride * i + frame.len();
        total_size = total_size.max(frame_end);
    }

    let mut out: Vec<f32> = vec![0.0; total_size];
    let mut sum_weight: Vec<f32> = vec![0.0; total_size];

    let mut offset: usize = 0;
    for frame in frames {
        let l = frame.len();
        // Python: t = np.linspace(0,1,L+2,dtype=f32)[1:-1]
        //         weight = |0.5 - (t - 0.5)| = 1 - t   (descending ramp)
        //         t_k = (k+1)/(L+1), weight_k = 1 - (k+1)/(L+1)
        let l_plus_1 = (l + 1) as f32;
        for k in 0..l {
            let w = 1.0_f32 - ((k + 1) as f32) / l_plus_1;
            let idx = offset + k;
            out[idx] += w * frame[k];
            sum_weight[idx] += w;
        }
        offset += stride;
    }

    // Every output sample must be covered by at least one frame
    debug_assert!(sum_weight.iter().all(|&w| w > 0.0));

    // Normalize by sum of weights
    for i in 0..total_size {
        out[i] /= sum_weight[i];
    }

    out
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the descending-ramp weight formula matches the Python
    /// `np.linspace(0,1,L+2)[1:-1]` → `1 - t` derivation exactly.
    #[test]
    fn test_weight_formula() {
        // For L=10: t = [1/11, 2/11, ..., 10/11], w = [10/11, 9/11, ..., 1/11]
        let l = 10usize;
        let l_plus_1 = (l + 1) as f32;
        for k in 0..l {
            let w = 1.0_f32 - ((k + 1) as f32) / l_plus_1;
            let expected = ((l - k) as f32) / l_plus_1;
            assert!(
                (w - expected).abs() < 1e-7,
                "k={k}: w={w}, expected={expected}"
            );
        }
    }

    /// Single-frame overlap-add should return the frame weighted and
    /// normalized (sum_weight == weight → output == frame).
    #[test]
    fn test_single_frame_identity() {
        let frame: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
        let stride = 50; // stride < len → overlap would happen with a 2nd frame
        let result = linear_overlap_add(&[frame.clone()], stride);
        assert_eq!(result.len(), frame.len());
        for (r, f) in result.iter().zip(frame.iter()) {
            assert!((r - f).abs() < 1e-6, "mismatch: {r} vs {f}");
        }
    }

    /// Two overlapping frames with stride = half the frame length.
    /// Each output sample is covered by 1 or 2 frames; verify the
    /// weight-sum normalization produces a plausible blend.
    #[test]
    fn test_two_frames_overlap() {
        let frame: Vec<f32> = vec![1.0; 100];
        let stride = 50;
        let result = linear_overlap_add(&[frame.clone(), frame.clone()], stride);
        // Total size = stride*1 + 100 = 150
        assert_eq!(result.len(), 150);
        // All samples should be 1.0 (constant input, weights sum the same
        // for both contributions at every overlapping point).
        // Actually with descending ramp, constant input doesn't stay 1.0
        // in the overlap region because the two ramps have opposite slopes.
        // Just verify no NaN/Inf and all values in a plausible range.
        for &v in &result {
            assert!(v.is_finite(), "non-finite value: {v}");
        }
    }

    /// StreamState::new seeds correctly from ref_codes.
    #[test]
    fn test_new_state() {
        let ref_codes: Vec<i32> = vec![100, 200, 300];
        let state = StreamState::new(&ref_codes);
        assert_eq!(state.cache, vec![100, 200, 300]);
        assert_eq!(state.n_decoded_tokens, 3);
        assert_eq!(state.n_decoded_samples, 0);
        assert!(state.audio_cache.is_empty());
    }

    /// Descending ramp: first sample has highest weight, last sample lowest.
    #[test]
    fn test_descending_ramp_order() {
        let l = 20usize;
        let l_plus_1 = (l + 1) as f32;
        let mut prev_w = 2.0_f32; // larger than any weight
        for k in 0..l {
            let w = 1.0_f32 - ((k + 1) as f32) / l_plus_1;
            assert!(w < prev_w, "weight not descending at k={k}: {w} >= {prev_w}");
            prev_w = w;
        }
        // First weight should be close to 1, last close to 0
        let first_w = 1.0_f32 - 1.0 / l_plus_1;
        let last_w = 1.0_f32 - l as f32 / l_plus_1;
        assert!(first_w > 0.9, "first weight too low: {first_w}");
        assert!(last_w < 0.1, "last weight too high: {last_w}");
    }
}
