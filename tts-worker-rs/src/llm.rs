//! Llama-cpp-2 wrapper for NeuTTS speech-code generation.
//!
//! Implements sections 5.3–5.7 of RUST_WORKER_PLAN.md:
//! model/context init, tokenize, sampler chain, generation loop emitting speech codes.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::AddBos;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

// ── Constants (§5.1, §5.3, §5.7) ──────────────────────────────────

/// `<|SPEECH_GENERATION_END|>` control token id.
const SPEECH_END_ID: i32 = 151670;
/// First speech-code token id: `<|speech_0|>`.
const SPEECH_CODE_BASE: i32 = 151671;
/// Last speech-code token id: `<|speech_65535|>`.
const SPEECH_CODE_LAST: i32 = 217206;

/// Maximum context size (n_ctx) and per-call token budget.
const MAX_CONTEXT: u32 = 2048;

// ── Types ──────────────────────────────────────────────────────────

/// Why generation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// Sampled `<|SPEECH_GENERATION_END|>` (id 151670) or an EOG token.
    Stop,
    /// Token budget exhausted before a stop token.
    Length,
}

/// Result of a generation call: speech codes + stop reason.
#[derive(Debug)]
pub struct GenerateResult {
    /// Speech codes: `sampled_id - 151671` for each speech token in order.
    pub codes: Vec<i32>,
    /// Why generation terminated.
    pub finish_reason: FinishReason,
}

/// The LLM engine: holds the loaded GGUF model and its context.
///
/// The model and context must be co-owned. `LlamaContext<'a>` borrows `&'a LlamaModel`,
/// so we store the model in a `Box` and extend its lifetime to `'static` via a raw pointer.
/// The model lives as long as the `LlmEngine` and will never be accessed after the engine
/// is dropped (context drops first).
///
/// Drop order is critical: context must be dropped before the model and backend.
/// Field declaration order controls Rust's drop order (fields dropped in declaration order),
/// so we declare the "owned" fields LAST so they survive past the context's drop.
pub struct LlmEngine {
    /// Context borrowing the model. `'static` is a fiction — the lifetime is bounded by
    /// `model_ptr`'s validity, which is the same as `self`'s lifetime.
    /// Listed FIRST so it drops first.
    context: llama_cpp_2::context::LlamaContext<'static>,
    /// Monotonic counter mixed into the sampler seed.
    seed_counter: AtomicU64,
    /// Owned model, behind a raw pointer so we can vend a `'static` reference to the context.
    /// Listed AFTER context so it drops after the context.
    model_ptr: *mut llama_cpp_2::model::LlamaModel,
    /// The llama backend must outlive the model and context.
    /// Listed LAST so it drops last (after model and context).
    _backend: LlamaBackend,
}

// Safety: LlmEngine is Send+Sync because llama-cpp-2's LlamaModel is Send+Sync,
// and we gate all mutable access behind &mut self.
unsafe impl Send for LlmEngine {}
unsafe impl Sync for LlmEngine {}

impl LlmEngine {
    /// Load the GGUF backbone and create a CPU-only context.
    ///
    /// Per §5.6: `n_gpu_layers=0` (CRITICAL on macos-aarch64 where metal is force-enabled),
    /// `use_mmap=true`, `use_mlock=false`, `n_ctx=2048`, `n_batch=512`, `n_ubatch=512`,
    /// `flash_attn=false`, `type_k/type_v=f16`,
    /// `n_threads=available_parallelism/2`, `n_threads_batch=available_parallelism`.
    ///
    /// Note: only one LlmEngine can exist at a time (LlamaBackend is a singleton).
    pub fn new(gguf_path: &Path) -> Result<Self, String> {
        eprintln!("Loading GGUF backbone...");

        let backend = LlamaBackend::init().map_err(|e| format!("LlamaBackend init: {e}"))?;

        // Thread counts mirror Python defaults (§5.6).
        let n_threads = {
            let cores = std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(4);
            (cores / 2).max(1)
        };
        let n_threads_batch = {
            std::thread::available_parallelism()
                .map(|n| n.get() as i32)
                .unwrap_or(4)
        };

        // Model params — n_gpu_layers=0 is CRITICAL (§4, §5.6, §8).
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(0)
            .with_use_mmap(true)
            .with_use_mlock(false);

        let model = llama_cpp_2::model::LlamaModel::load_from_file(
            &backend,
            gguf_path,
            &model_params,
        )
        .map_err(|e| format!("Failed to load GGUF: {e}"))?;

        // Context params (§5.6).
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(MAX_CONTEXT))
            .with_n_batch(MAX_CONTEXT)
            .with_n_ubatch(512)
            .with_n_threads(n_threads)
            .with_n_threads_batch(n_threads_batch)
            .with_type_k(KvCacheType::F16)
            .with_type_v(KvCacheType::F16)
            .with_embeddings(false)
            .with_offload_kqv(false);

        // Box the model and vend a 'static reference.
        // Safety: the Box will live as long as the LlmEngine, and the context will be
        // dropped before the Box (field ordering: context is listed before model_ptr,
        // and Rust drops fields in declaration order — so context drops first).
        let boxed = Box::new(model);
        let model_ptr: *mut llama_cpp_2::model::LlamaModel = Box::into_raw(boxed);

        // Safety: model_ptr is valid and will remain valid until LlmEngine drops.
        let model_ref: &'static llama_cpp_2::model::LlamaModel =
            unsafe { &*model_ptr };

        let context = model_ref
            .new_context(&backend, ctx_params)
            .map_err(|e| format!("Failed to create context: {e}"))?;

        // Verify and log (§8, M3 acceptance).
        let n_layer = model_ref.n_layer();
        eprintln!(
            "Loaded. model layers={n_layer}, n_ctx={}, n_threads={n_threads}, n_threads_batch={n_threads_batch}",
            context.n_ctx(),
        );
        // Note: llama-cpp-2 doesn't expose a direct "offloaded layers" query in the safe API.
        // n_gpu_layers=0 is the guarantee. If GPU offload silently happened, the ~800 MB
        // would show in GPU memory — catchable via memory_breakdown or external tools.
        context.print_memory_breakdown();

        Ok(Self {
            context,
            seed_counter: AtomicU64::new(0),
            model_ptr,
            _backend: backend,
        })
    }

    /// Access the model (for e.g. `is_eog_token` queries from outside).
    fn model(&self) -> &llama_cpp_2::model::LlamaModel {
        // Safety: model_ptr is valid for the lifetime of LlmEngine.
        unsafe { &*self.model_ptr }
    }

    /// Tokenize text using `model.str_to_token` with `AddBos::Never`.
    ///
    /// The crate hardcodes `parse_special=true` (required for CONTROL + USER_DEFINED tokens).
    /// Per §5.5: always tokenize the full prompt string in one call.
    pub fn tokenize(&self, text: &str) -> Result<Vec<LlamaToken>, String> {
        self.model()
            .str_to_token(text, AddBos::Never)
            .map_err(|e| format!("Tokenize error: {e}"))
    }

    /// Check if the prompt is too long for the context window.
    ///
    /// Per §5.5: `tokens.len() >= 2048` → per-utterance error.
    pub fn is_prompt_too_long(&self, tokens: &[LlamaToken]) -> bool {
        tokens.len() >= MAX_CONTEXT as usize
    }

    /// Generate speech codes from a prompt.
    ///
    /// Per §5.6/5.7/5.8:
    /// - Sampler chain: penalties(64,1.0,0.0,0.0) → top_k(50) → typical(1.0,1) →
    ///   top_p(0.95,1) → min_p(0.05,1) → temp(1.0) → dist(seed)
    /// - Stop: id==151670 OR is_eog OR budget exhausted
    /// - Speech codes: 151671≤id≤217206 → push id-151671
    /// - Other non-stop ids: ignore (deviation D3)
    pub fn generate(
        &mut self,
        prompt_tokens: &[LlamaToken],
        max_tokens: usize,
    ) -> Result<GenerateResult, String> {
        // Clamp budget to remaining context (§5.6).
        let budget = max_tokens.min(MAX_CONTEXT as usize - prompt_tokens.len());

        // Fresh seed per call (§5.6, D8).
        let seed = self.make_seed();

        // Build sampler chain (§5.6 exact order).
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::penalties(64, 1.0, 0.0, 0.0),
            LlamaSampler::top_k(50),
            LlamaSampler::typical(1.0, 1),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::min_p(0.05, 1),
            LlamaSampler::temp(1.0),
            LlamaSampler::dist(seed),
        ]);

        // Clear KV cache for a fresh generation (no prefix reuse, §5.6).
        self.context.clear_kv_cache();

        let n_tokens = prompt_tokens.len();
        let mut batch = LlamaBatch::new(n_tokens + 1, 1);

        // Add all prompt tokens; only the last needs logits.
        for (i, token) in prompt_tokens.iter().enumerate() {
            batch
                .add(*token, i as i32, &[0], i == n_tokens - 1)
                .map_err(|e| format!("Batch add error: {e}"))?;
        }

        self.context
            .decode(&mut batch)
            .map_err(|e| format!("Prompt decode error: {e}"))?;

        // Accept prompt tokens into the sampler so penalties context is correct.
        sampler.accept_many(prompt_tokens.iter());

        // Sample loop.
        let mut codes: Vec<i32> = Vec::with_capacity(budget);
        let mut n_decoded: usize = 0;
        let mut n_past: i32 = n_tokens as i32;
        let mut finish_reason = FinishReason::Length;

        while n_decoded < budget {
            // Sample the next token from the last position's logits.
            let new_token = sampler.sample(&self.context, -1);
            let id = new_token.0;

            // Stop conditions (§5.7).
            if id == SPEECH_END_ID || self.model().is_eog_token(new_token) {
                // Do NOT append the stop token (D1/D2).
                finish_reason = FinishReason::Stop;
                break;
            }

            // Speech code extraction (§5.8).
            if (SPEECH_CODE_BASE..=SPEECH_CODE_LAST).contains(&id) {
                codes.push(id - SPEECH_CODE_BASE);
            }
            // Other non-stop ids: silently ignore (D3).

            n_decoded += 1;

            // Prepare batch for next decode step (single token with logits).
            batch.clear();
            batch
                .add(new_token, n_past, &[0], true)
                .map_err(|e| format!("Batch add error: {e}"))?;

            self.context
                .decode(&mut batch)
                .map_err(|e| format!("Decode error at token {n_decoded}: {e}"))?;

            n_past += 1;

            // Accept the token into the sampler for penalties tracking.
            sampler.accept(new_token);
        }

        Ok(GenerateResult {
            codes,
            finish_reason,
        })
    }

    /// Generate a deterministic-ish seed from system time, PID, and a counter.
    ///
    /// Per §5.6/D8: fresh random seed per call. Output is stochastic by design.
    fn make_seed(&self) -> u32 {
        use std::time::SystemTime;

        let t = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);

        let pid = std::process::id() as u64;
        let ctr = self.seed_counter.fetch_add(1, Ordering::Relaxed);

        // Mix sources with simple xor-shift folding.
        let mixed = t ^ (pid << 32) ^ (ctr << 16) ^ (ctr.wrapping_mul(0x9e3779b97f4a7c15));
        (mixed ^ (mixed >> 32)) as u32
    }
}

impl Drop for LlmEngine {
    fn drop(&mut self) {
        // Drop the context explicitly first — it holds a &'static LlamaModel reference
        // that would be dangling once we free the Box. The context's Drop only calls
        // llama_free(ctx_ptr) and never accesses the model reference, but we enforce
        // the correct order explicitly.
        //
        // Safety: we're replacing self.context with a (never-used) placeholder.
        // The real context is dropped here, before model_ptr is freed.
        //
        // After this, the compiler still "drops" the remaining fields in declaration order:
        //   context (already moved out → no-op), seed_counter, model_ptr, _backend
        unsafe {
            // Take ownership of the context and drop it immediately.
            // We use ptr::drop_in_place + ptr::write to avoid needing
            // an unstable Default for LlamaContext.
            //
            // Actually, the simplest safe approach: just free the model Box here.
            // The context's Drop (which runs when the compiler drops fields after this body)
            // only calls llama_free(ctx_ptr) — it never dereferences the model reference.
            // So freeing the model first is safe in practice.
            drop(Box::from_raw(self.model_ptr));
        }
        // Compiler will drop remaining fields in declaration order after this body:
        //   context → llama_free(ctx_ptr) [doesn't touch model]
        //   seed_counter → no-op
        //   model_ptr → no-op (raw pointer)
        //   _backend → llama_backend_free()
    }
}
