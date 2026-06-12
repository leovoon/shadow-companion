//! Audio output: `AudioSink` trait, `CpalSink` (real playback), `WavSink` (test mode).
//!
//! Plan §5.11 — per-utterance sink lifecycle:
//! - Fresh sink per utterance.
//! - `push_samples` feeds f32 chunks.
//! - `push_tail_pad` appends 0.15 s of zeros (3600 samples @ 24 kHz).
//! - `drain_and_finish` waits for playback to complete, then tears down.
//!
//! `TTS_TEST_WAV` env selects `WavSink`, else `CpalSink`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Producer, RingBuffer};

const SAMPLE_RATE: u32 = 24_000;
const TAIL_PAD_SAMPLES: usize = 3600; // 0.15 s @ 24 kHz
const RING_CAPACITY: usize = 1_440_000; // ~60 s @ 24 kHz
const BLOCK_SLEEP: Duration = Duration::from_millis(10);
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// AudioSink trait
// ---------------------------------------------------------------------------

pub trait AudioSink {
    fn push_samples(&mut self, samples: &[f32]);
    fn push_tail_pad(&mut self);
    fn drain_and_finish(&mut self, total_duration_secs: f64) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// CpalSink — real-time playback via cpal + rtrb ring buffer
// ---------------------------------------------------------------------------

pub struct CpalSink {
    producer: Option<Producer<f32>>,
    stream: Option<cpal::Stream>,
    term_flag: Arc<AtomicBool>,
}

impl CpalSink {
    /// Create a new CpalSink. Stream is lazily started on first `push_samples`.
    /// `term_flag` is checked during drain; set it on SIGTERM.
    pub fn new(term_flag: Arc<AtomicBool>) -> Self {
        Self {
            producer: None,
            stream: None,
            term_flag,
        }
    }

    /// Start the cpal output stream with an rtrb ring buffer.
    fn start_stream(&mut self) -> Result<Producer<f32>, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No audio output device found".to_string())?;

        let desired_config = cpal::StreamConfig {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Fixed(1024),
        };

        let (stream, producer) =
            build_stream_with_fallback(&device, &desired_config, self.term_flag.clone())?;

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {}", e))?;

        self.stream = Some(stream);
        Ok(producer)
    }
}

/// Try the desired config; on failure, fall back to device default config
/// (with trivial resampling if the device rate ≠ 24 kHz).
fn build_stream_with_fallback(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    term_flag: Arc<AtomicBool>,
) -> Result<(cpal::Stream, Producer<f32>), String> {
    match build_stream(device, config, term_flag.clone()) {
        ok @ Ok(_) => ok,
        Err(primary_err) => {
            eprintln!(
                "Primary audio config failed ({}), trying device default",
                primary_err
            );
            let default_config = device
                .default_output_config()
                .map_err(|e| format!("No default output config: {}", e))?;
            let fallback_config = cpal::StreamConfig {
                channels: default_config.channels(),
                sample_rate: default_config.sample_rate(),
                buffer_size: cpal::BufferSize::Default,
            };
            build_stream(device, &fallback_config, term_flag)
        }
    }
}

/// Create an rtrb ring, build a cpal stream consuming from it, and return
/// `(stream, producer)`. The callback zero-fills on underrun (D5). If
/// `device_rate ≠ 24000`, performs trivial nearest-neighbour resampling.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    _term_flag: Arc<AtomicBool>,
) -> Result<(cpal::Stream, Producer<f32>), String> {
    let (producer, mut consumer) = RingBuffer::<f32>::new(RING_CAPACITY);

    let device_rate = config.sample_rate as f64;
    let src_rate: f64 = SAMPLE_RATE as f64;
    let needs_resample = (device_rate - src_rate).abs() > 1.0;
    let ratio = src_rate / device_rate; // src samples per device sample
    let channels = config.channels as usize;

    struct ResampleState {
        frac_pos: f64,
        held: f32,
        have_held: bool,
    }
    let rs = std::cell::RefCell::new(ResampleState {
        frac_pos: 0.0,
        held: 0.0,
        have_held: false,
    });

    let stream = device
        .build_output_stream::<f32, _, _>(
            config.clone(),
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                if !needs_resample {
                    for frame in data.chunks_exact_mut(channels) {
                        let s = match consumer.pop() {
                            Ok(v) => v,
                            Err(_) => 0.0,
                        };
                        for ch in frame.iter_mut() {
                            *ch = s;
                        }
                    }
                } else {
                    let mut st = rs.borrow_mut();
                    for frame in data.chunks_exact_mut(channels) {
                        st.frac_pos += ratio;
                        while st.frac_pos >= 1.0 {
                            st.frac_pos -= 1.0;
                            match consumer.pop() {
                                Ok(v) => {
                                    st.held = v;
                                    st.have_held = true;
                                }
                                Err(_) => {
                                    st.have_held = false;
                                    break;
                                }
                            }
                        }
                        let s = if st.have_held { st.held } else { 0.0 };
                        for ch in frame.iter_mut() {
                            *ch = s;
                        }
                    }
                }
            },
            |err: cpal::Error| {
                eprintln!("Audio stream error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("Failed to build audio stream: {}", e))?;

    Ok((stream, producer))
}

impl AudioSink for CpalSink {
    fn push_samples(&mut self, samples: &[f32]) {
        if self.producer.is_none() {
            match self.start_stream() {
                Ok(prod) => self.producer = Some(prod),
                Err(e) => {
                    eprintln!("CpalSink: failed to start stream: {}", e);
                    return;
                }
            }
        }

        let producer = match self.producer.as_mut() {
            Some(p) => p,
            None => return,
        };

        let mut remaining = samples;
        while !remaining.is_empty() {
            let (_, rest) = producer.push_partial_slice(remaining);
            if rest.len() == remaining.len() {
                thread::sleep(BLOCK_SLEEP);
            }
            remaining = rest;
        }
    }

    fn push_tail_pad(&mut self) {
        let zeros = [0.0f32; TAIL_PAD_SAMPLES];
        self.push_samples(&zeros);
    }

    fn drain_and_finish(&mut self, total_duration_secs: f64) -> Result<(), String> {
        let timeout_secs = (total_duration_secs + 5.0).max(10.0);
        let deadline = std::time::Instant::now() + Duration::from_secs_f64(timeout_secs);

        while std::time::Instant::now() < deadline && !self.term_flag.load(Ordering::Relaxed) {
            if let Some(ref producer) = self.producer {
                if producer.slots() >= RING_CAPACITY - 1024 {
                    thread::sleep(DRAIN_POLL_INTERVAL);
                    break;
                }
            } else {
                break;
            }
            thread::sleep(DRAIN_POLL_INTERVAL);
        }

        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
            drop(stream);
        }
        self.producer = None;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WavSink — write to WAV file (TTS_TEST_WAV mode)
// ---------------------------------------------------------------------------

pub struct WavSink {
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    path: std::path::PathBuf,
}

impl WavSink {
    pub fn new(path: &std::path::Path) -> Result<Self, String> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let writer = hound::WavWriter::create(path, spec)
            .map_err(|e| format!("Failed to create WAV '{}': {}", path.display(), e))?;
        Ok(Self {
            writer: Some(writer),
            path: path.to_path_buf(),
        })
    }
}

impl AudioSink for WavSink {
    fn push_samples(&mut self, samples: &[f32]) {
        if let Some(ref mut writer) = self.writer {
            for &s in samples {
                if let Err(e) = writer.write_sample(s) {
                    eprintln!("WavSink write error: {}", e);
                    return;
                }
            }
        }
    }

    fn push_tail_pad(&mut self) {
        let zeros = [0.0f32; TAIL_PAD_SAMPLES];
        self.push_samples(&zeros);
    }

    fn drain_and_finish(&mut self, _total_duration_secs: f64) -> Result<(), String> {
        if let Some(writer) = self.writer.take() {
            writer
                .finalize()
                .map_err(|e| format!("Failed to finalize WAV '{}': {}", self.path.display(), e))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sink factory — selects CpalSink or WavSink based on TTS_TEST_WAV env
// ---------------------------------------------------------------------------

/// Create the appropriate sink. If `TTS_TEST_WAV` is set, creates a `WavSink`
/// writing to that path; otherwise creates a `CpalSink` for real playback.
/// `term_flag` is only used by `CpalSink` for SIGTERM-aware drain.
pub fn create_sink(term_flag: Arc<AtomicBool>) -> Box<dyn AudioSink> {
    if let Ok(wav_path) = std::env::var("TTS_TEST_WAV") {
        let path = std::path::PathBuf::from(&wav_path);
        match WavSink::new(&path) {
            Ok(sink) => {
                eprintln!("WavSink mode: writing to {}", wav_path);
                return Box::new(sink);
            }
            Err(e) => {
                eprintln!("WavSink creation failed ({}), falling back to CpalSink", e);
            }
        }
    }
    Box::new(CpalSink::new(term_flag))
}
