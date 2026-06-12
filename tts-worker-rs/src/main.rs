mod audio;
mod codec;
mod espeak;
mod llm;
mod paths;
mod phonemize;
mod playlog;
mod protocol;
mod stream;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── SIGTERM flag ───────────────────────────────────────────────────────────

static SIGTERM: AtomicBool = AtomicBool::new(false);

fn install_sigterm_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigterm_handler as *const () as usize;
        // No SA_RESTART — so blocked read() returns EINTR
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

extern "C" fn sigterm_handler(_sig: libc::c_int) {
    SIGTERM.store(true, Ordering::Relaxed);
}

// ── Ref codes loader ───────────────────────────────────────────────────────

fn load_ref_codes(path: &std::path::Path) -> Result<Vec<i32>, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let npy = npyz::NpyFile::new(file)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let shape = npy.shape().to_vec();
    let codes: Vec<i32> = npy.into_vec::<i32>()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    eprintln!("ref_codes: shape={shape:?}, len={}", codes.len());
    Ok(codes)
}

// ── State.json check ───────────────────────────────────────────────────────

fn check_state_json() -> Result<(), String> {
    let path = paths::state_json_path();
    let txt = fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let val: serde_json::Value = serde_json::from_str(&txt)
        .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let provider = val.get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if provider != "neutts" {
        return Err(format!("state.json provider={provider} != neutts — refusing to start"));
    }
    Ok(())
}

// ── Prewarm (non-streaming) ────────────────────────────────────────────────

fn prewarm(
    espeak: &espeak::Espeak,
    engine: &mut llm::LlmEngine,
    codec: &codec::CodecDecoder,
    ref_codes: &[i32],
    ref_text_phones: &str,
) -> Result<(), String> {
    eprintln!("Prewarming with \"Ready.\"...");
    let input_phones = phonemize::phonemize_to_phones(espeak, "Ready.");
    let prompt = build_prompt(ref_text_phones, &input_phones, ref_codes);
    let tokens = engine.tokenize(&prompt)?;
    if engine.is_prompt_too_long(&tokens) {
        return Err("Prewarm prompt too long".to_string());
    }
    let result = engine.generate(&tokens, 2048 - tokens.len())?;
    if result.codes.is_empty() {
        return Err("Prewarm generated no speech codes".to_string());
    }
    // Decode all codes in one window, discard audio
    let all_codes = &result.codes;
    if all_codes.len() >= 2 {
        let _ = codec.decode_window(all_codes)?;
    }
    eprintln!("Prewarm done ({} codes generated).", result.codes.len());
    Ok(())
}

// ── Prompt builder ──────────────────────────────────────────────────────────

fn build_prompt(ref_phones: &str, input_phones: &str, ref_codes: &[i32]) -> String {
    let codes_str: String = ref_codes
        .iter()
        .map(|c| format!("<|speech_{}|>", c))
        .collect();
    format!(
        "user: Convert the text to speech:<|TEXT_PROMPT_START|>{} {}<|TEXT_PROMPT_END|>\nassistant:<|SPEECH_GENERATION_START|>{}",
        ref_phones, input_phones, codes_str
    )
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // ── Debug subcommand: --phonemize ──
    if args.len() > 2 && args[1] == "--phonemize" {
        let espeak_lib = match paths::resolve_espeak_lib() {
            Ok(p) => p,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        };
        let espeak_data = match paths::resolve_espeak_data() {
            Ok(p) => p,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        };
        let espeak = unsafe {
            match espeak::Espeak::new(&espeak_lib, &espeak_data) {
                Ok(e) => e,
                Err(e) => { eprintln!("{e}"); std::process::exit(1); }
            }
        };
        let result = phonemize::phonemize_to_phones(&espeak, &args[2]);
        println!("{result}");
        return;
    }

    // ── Debug subcommand: --tokenize ──
    if args.len() > 2 && args[1] == "--tokenize" {
        let gguf_path = match paths::resolve_gguf_path() {
            Ok(p) => p,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        };
        let engine = match llm::LlmEngine::new(&gguf_path) {
            Ok(e) => e,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        };
        let tokens = match engine.tokenize(&args[2]) {
            Ok(t) => t,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        };
        let ids: Vec<i32> = tokens.iter().map(|t| t.0).collect();
        println!("{:?}", ids);
        return;
    }

    // ── Normal mode ──

    // SIGTERM handler BEFORE anything else
    install_sigterm_handler();
    let term_flag = Arc::new(AtomicBool::new(false));
    // Mirror SIGTERM to the per-utterance term flag
    let term_flag_clone = term_flag.clone();
    std::thread::spawn(move || {
        // Spin-poll SIGTERM and propagate to the Arc<AtomicBool>
        loop {
            if SIGTERM.load(Ordering::Relaxed) {
                term_flag_clone.store(true, Ordering::Relaxed);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    // Check state.json provider
    if let Err(e) = check_state_json() {
        eprintln!("{e}");
        std::process::exit(1);
    }

    // Load ref_codes.npy
    let ref_codes_path = paths::ref_codes_path();
    let ref_codes = match load_ref_codes(&ref_codes_path) {
        Ok(c) => c,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    // Load ref_text.txt
    let ref_text_path = paths::ref_text_path();
    let ref_text = match fs::read_to_string(&ref_text_path) {
        Ok(t) => t.trim().to_string(),
        Err(e) => { eprintln!("cannot read {}: {e}", ref_text_path.display()); std::process::exit(1); }
    };

    // Init espeak
    let espeak_lib = match paths::resolve_espeak_lib() {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    let espeak_data = match paths::resolve_espeak_data() {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    let espeak = unsafe {
        match espeak::Espeak::new(&espeak_lib, &espeak_data) {
            Ok(e) => e,
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        }
    };

    // Init ONNX codec
    let onnx_path = match paths::resolve_onnx_path() {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    let codec = match codec::CodecDecoder::new(&onnx_path) {
        Ok(c) => c,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    // Init LLM (n_gpu_layers=0!)
    let gguf_path = match paths::resolve_gguf_path() {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    let mut engine = match llm::LlmEngine::new(&gguf_path) {
        Ok(e) => e,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };

    // Phonemize ref_text
    let ref_text_phones = phonemize::phonemize_to_phones(&espeak, &ref_text);

    // Prewarm
    if let Err(e) = prewarm(&espeak, &mut engine, &codec, &ref_codes, &ref_text_phones) {
        eprintln!("Prewarm failed: {e}");
        std::process::exit(1);
    }

    // Print WORKER_READY + flush
    protocol::print_worker_ready();

    // ── Stdin loop ──
    while !SIGTERM.load(Ordering::Relaxed) {
        let line = match protocol::read_stdin_line() {
            Some(l) => l,
            None => break, // EOF
        };

        let text = protocol::parse_text_input(&line);
        if text.is_empty() {
            continue;
        }

        // Per-utterance error containment
        if let Err(e) = process_utterance(
            &espeak,
            &mut engine,
            &codec,
            &ref_codes,
            &ref_text_phones,
            &text,
            &term_flag,
        ) {
            protocol::print_error(&e);
            continue;
        }
    }

    // Clean exit on SIGTERM or EOF
    std::process::exit(0);
}

// ── Utterance processing ───────────────────────────────────────────────────

fn process_utterance(
    espeak: &espeak::Espeak,
    engine: &mut llm::LlmEngine,
    codec: &codec::CodecDecoder,
    ref_codes: &[i32],
    ref_text_phones: &str,
    input_text: &str,
    term_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    // Reset term flag for this utterance
    term_flag.store(false, Ordering::Relaxed);

    // Phonemize input
    let input_phones = phonemize::phonemize_to_phones(espeak, input_text);

    // Build prompt
    let prompt = build_prompt(ref_text_phones, &input_phones, ref_codes);

    // Tokenize
    let tokens = engine.tokenize(&prompt)?;
    if engine.is_prompt_too_long(&tokens) {
        return Err(format!("prompt too long ({} tokens)", tokens.len()));
    }

    // Generate codes with streaming decode
    let budget = 2048 - tokens.len();
    let gen_result = engine.generate(&tokens, budget)?;

    // Check SIGTERM
    if SIGTERM.load(Ordering::Relaxed) {
        return Ok(());
    }

    if gen_result.codes.is_empty() {
        return Err("no speech tokens generated".to_string());
    }

    // Stream-decode the generated codes using StreamState
    let mut stream_state = stream::StreamState::new(ref_codes);
    let mut all_audio: Vec<f32> = Vec::new();

    for code in &gen_result.codes {
        // Check SIGTERM during decode
        if SIGTERM.load(Ordering::Relaxed) {
            return Ok(());
        }

        if let Some(chunk) = stream_state.push_code(*code, codec)? {
            all_audio.extend_from_slice(&chunk);
        }
    }

    // Final flush
    if let Some(chunk) = stream_state.final_flush(codec)? {
        all_audio.extend_from_slice(&chunk);
    }

    // Check SIGTERM
    if SIGTERM.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Total samples (excluding tail pad)
    let total_samples = all_audio.len();
    let total_duration = total_samples as f64 / 24000.0;

    if total_duration <= 0.0 {
        return Err("zero audio duration".to_string());
    }

    // Play audio
    let mut sink = audio::create_sink(term_flag.clone());
    sink.push_samples(&all_audio);
    sink.push_tail_pad();
    sink.drain_and_finish(total_duration)?;

    // Log play
    playlog::log_play(total_duration);

    // Print played duration
    protocol::print_played(total_duration);

    Ok(())
}
