use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::os::raw::c_void;
use std::path::Path;
use libloading::Library;

pub struct Espeak {
    _lib: Library,
    text_to_phonemes_fn: unsafe extern "C" fn(*mut *const c_void, i32, i32) -> *const c_char,
}

impl Espeak {
    /// Load espeak-ng dylib, initialize, set voice.
    /// `dylib_path` = libespeak-ng.dylib, `data_path` = espeak-ng-data directory.
    pub unsafe fn new(dylib_path: &Path, data_path: &Path) -> Result<Self, String> {
        std::env::set_var("ESPEAK_DATA_PATH", data_path);

        let lib = Library::new(dylib_path)
            .map_err(|e| format!("espeak dlopen {:?}: {}", dylib_path, e))?;

        let init_fn: unsafe extern "C" fn(i32, i32, *const c_char, i32) -> i32 =
            *lib.get(b"espeak_Initialize\0")
                .map_err(|e| format!("symbol espeak_Initialize: {}", e))?;

        let set_voice_fn: unsafe extern "C" fn(*const c_char) -> i32 =
            *lib.get(b"espeak_SetVoiceByName\0")
                .map_err(|e| format!("symbol espeak_SetVoiceByName: {}", e))?;

        let text_to_phonemes_fn: unsafe extern "C" fn(*mut *const c_void, i32, i32) -> *const c_char =
            *lib.get(b"espeak_TextToPhonemes\0")
                .map_err(|e| format!("symbol espeak_TextToPhonemes: {}", e))?;

        // espeak_Initialize(AUDIO_OUTPUT_SYNCHRONOUS=0x02, buflength=0, path=NULL, options=0)
        let sample_rate = init_fn(0x02, 0, std::ptr::null(), 0);
        if sample_rate <= 0 {
            return Err(format!("espeak_Initialize failed: {}", sample_rate));
        }

        // espeak_SetVoiceByName("gmw/en-us") → EE_OK=0
        let voice = b"gmw/en-us\0";
        let rc = set_voice_fn(voice.as_ptr() as *const c_char);
        if rc != 0 {
            return Err(format!("espeak_SetVoiceByName(gmw/en-us) failed: {}", rc));
        }

        Ok(Self {
            _lib: lib,
            text_to_phonemes_fn,
        })
    }

    /// Phonemize text via espeak-ng. Returns IPA phonemes with '_' separators, words joined by ' '.
    pub fn text_to_phonemes(&self, text: &str) -> String {
        let c_text = CString::new(text).expect("text contains NUL");
        let mut ptr: *const c_void = c_text.as_ptr() as *const c_void;

        let mut parts: Vec<String> = Vec::new();

        // espeak consumes one sentence per call; loop until ptr is NULL
        while !ptr.is_null() {
            let phonemes = unsafe {
                (self.text_to_phonemes_fn)(&mut ptr, 1 /* UTF-8 */, 0x5F02 /* IPA + '_' separator */)
            };

            if phonemes.is_null() {
                break;
            }

            // Copy immediately — espeak reuses a static buffer
            let s = unsafe { CStr::from_ptr(phonemes) }
                .to_str()
                .unwrap_or("")
                .to_owned();

            if !s.is_empty() {
                parts.push(s);
            }
        }

        parts.join(" ")
    }
}

// espeak-ng has global state; NOT safe to send across threads or share.
// Compile-time enforcement: omit Send/Sync impls (default is !Send + !Sync).
