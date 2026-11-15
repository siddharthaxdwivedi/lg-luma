//! Minimal FFI layer over libvosk for streaming transcription.
//!
//! Vosk endpoints on its own: audio is pushed in as it arrives and
//! `vosk_recognizer_accept_waveform_s` returns 1 once it decides the utterance
//! ended, so nothing here buffers a whole clip. A recognizer shares the loaded
//! model's data and resets itself after every result.

use std::ffi::{CStr, CString, c_void};
use std::os::raw::{c_char, c_int};
use std::ptr::NonNull;

unsafe extern "C" {
    fn vosk_set_log_level(log_level: c_int);
    fn vosk_model_new(model_path: *const c_char) -> *mut c_void;
    fn vosk_model_free(model: *mut c_void);
    fn vosk_recognizer_new(model: *mut c_void, sample_rate: f32) -> *mut c_void;
    fn vosk_recognizer_free(recognizer: *mut c_void);
    fn vosk_recognizer_set_max_alternatives(recognizer: *mut c_void, max_alternatives: c_int);
    fn vosk_recognizer_set_words(recognizer: *mut c_void, words: c_int);
    fn vosk_recognizer_set_endpointer_delays(
        recognizer: *mut c_void,
        t_start_max: f32,
        t_end: f32,
        t_max: f32,
    );
    fn vosk_recognizer_accept_waveform_s(
        recognizer: *mut c_void,
        data: *const i16,
        length: c_int,
    ) -> c_int;
    fn vosk_recognizer_result(recognizer: *mut c_void) -> *const c_char;
    fn vosk_recognizer_final_result(recognizer: *mut c_void) -> *const c_char;
}

/// Trailing silence that ends an utterance once speech was heard.
pub const END_AFTER_SILENCE_SECS: f32 = 0.9;

/// Silence with no speech yet before Vosk reports an empty utterance.
const START_MAX_SECS: f32 = 5.0;

/// Hard cap on one utterance, so a noisy room cannot grow the decoder forever.
const MAX_UTTERANCE_SECS: f32 = 30.0;

/// A loaded Vosk model: the graph is loaded once and shared by every recognizer
/// created from it.
pub struct VoskEngine {
    model: NonNull<c_void>,
}

impl VoskEngine {
    /// Load the model directory that holds `am/`, `conf/`, `graph/` and friends.
    pub fn load(model_dir: &str) -> Result<Self, String> {
        let path =
            CString::new(model_dir).map_err(|_| "model directory contains NUL".to_string())?;
        // SAFETY: libvosk only reads the path string, the log level is set
        // before loading so the loader cannot flood the TV log, and the loaded
        // model is released exactly once in `Drop`.
        let model = unsafe {
            vosk_set_log_level(-1);
            NonNull::new(vosk_model_new(path.as_ptr()))
        };
        model
            .ok_or_else(|| format!("Vosk could not load the model in {model_dir}"))
            .map(|model| Self { model })
    }

    /// A recognizer for one utterance at a time. It shares the model's data, so
    /// creating one per utterance is cheap.
    pub fn recognizer(&self, sample_rate: f32) -> Result<Recognizer, String> {
        // SAFETY: `self.model` is alive while the recognizer is, and libvosk
        // reference-counts the model.
        let raw = unsafe { vosk_recognizer_new(self.model.as_ptr(), sample_rate) };
        let inner =
            NonNull::new(raw).ok_or_else(|| "Vosk could not create a recognizer".to_string())?;
        // SAFETY: `inner` was just created. Alternatives and word times are off
        // because only the plain text is used, and the delays bound how long a
        // single utterance may run.
        unsafe {
            vosk_recognizer_set_max_alternatives(inner.as_ptr(), 0);
            vosk_recognizer_set_words(inner.as_ptr(), 0);
            vosk_recognizer_set_endpointer_delays(
                inner.as_ptr(),
                START_MAX_SECS,
                END_AFTER_SILENCE_SECS,
                MAX_UTTERANCE_SECS,
            );
        }
        Ok(Recognizer { inner })
    }
}

impl Drop for VoskEngine {
    fn drop(&mut self) {
        // SAFETY: the model was created by `load` and is freed once.
        unsafe { vosk_model_free(self.model.as_ptr()) };
    }
}

/// Reads one utterance at a time from a stream of 16 kHz mono PCM.
pub struct Recognizer {
    inner: NonNull<c_void>,
}

impl Recognizer {
    /// Push a chunk of audio. Returns `Some(text)` once Vosk decided the
    /// utterance ended, `None` while it is still listening, or an error when
    /// libvosk failed and the caller should start a new recognizer.
    pub fn accept(&mut self, samples: &[i16]) -> Result<Option<String>, String> {
        if samples.is_empty() {
            return Ok(None);
        }
        // SAFETY: `samples` is a live slice of exactly `samples.len()` i16
        // samples, which is the length handed to libvosk.
        let status = unsafe {
            vosk_recognizer_accept_waveform_s(
                self.inner.as_ptr(),
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        match status {
            0 => Ok(None),
            1 => Ok(Some(self.result(vosk_recognizer_result))),
            _ => Err("Vosk could not accept audio".to_string()),
        }
    }

    /// Close a half-spoken utterance, for when the orb is closed early.
    pub fn finish(&mut self) -> String {
        self.result(vosk_recognizer_final_result)
    }

    fn result(&self, get: unsafe extern "C" fn(*mut c_void) -> *const c_char) -> String {
        // SAFETY: the recognizer is alive and the returned string is owned by
        // it, so the text is copied out before libvosk is called again.
        let raw = unsafe { get(self.inner.as_ptr()) };
        if raw.is_null() {
            return String::new();
        }
        // SAFETY: Vosk returns a NUL-terminated string.
        text_of(&unsafe { CStr::from_ptr(raw) }.to_string_lossy())
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        // SAFETY: the recognizer was created by `VoskEngine::recognizer` and is
        // freed once.
        unsafe { vosk_recognizer_free(self.inner.as_ptr()) };
    }
}

/// Vosk returns `{"text": "..."}`, and `{"text": ""}` for a silent utterance.
fn text_of(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(|text| text.trim().to_string())
        })
        .unwrap_or_default()
}
