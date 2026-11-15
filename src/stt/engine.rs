//! Minimal FFI layer over whisper.cpp for transcription.
//!
//! `whisper-rs`'s safe wrapper does not expose the Silero VAD controls or the
//! thread clamp used here, so the raw bindings are called directly. Decoding is
//! left unconstrained: the transcript is matched against local commands
//! afterwards, and anything that is not a command goes to the cloud assistant.

use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;

use whisper_rs_sys as sys;

/// whisper.cpp works at 16 kHz mono.
pub const SAMPLE_RATE: usize = 16_000;

/// Shortest clip worth decoding (~0.3 s).
const MIN_SAMPLES: usize = SAMPLE_RATE / 3;

/// A loaded whisper.cpp context and its optional Silero VAD model.
pub struct Engine {
    ctx: *mut sys::whisper_context,
    state: *mut sys::whisper_state,
    vad: *mut sys::whisper_vad_context,
    vad_params: sys::whisper_vad_params,
    language: CString,
    n_threads: c_int,
}

impl Engine {
    /// Load a GGML model and the optional Silero VAD model. `n_threads` is
    /// capped so the TV keeps a core free.
    pub fn load(model: &str, vad_model: Option<&str>, n_threads: c_int) -> Result<Self, String> {
        let model = CString::new(model).map_err(|_| "model path contains NUL".to_string())?;
        let language = CString::new("en").map_err(|_| "language contains NUL".to_string())?;

        // SAFETY: default parameters are plain data and the path is a valid C string.
        let ctx = unsafe {
            let mut params = sys::whisper_context_default_params();
            params.use_gpu = false;
            sys::whisper_init_from_file_with_params(model.as_ptr(), params)
        };
        if ctx.is_null() {
            return Err(format!(
                "whisper.cpp could not load {}",
                model.to_string_lossy()
            ));
        }

        // SAFETY: `ctx` is a live context owned by the returned engine.
        let state = unsafe { sys::whisper_init_state(ctx) };
        if state.is_null() {
            // SAFETY: `ctx` was created above and is not used again.
            unsafe { sys::whisper_free(ctx) };
            return Err("whisper.cpp could not create a decoding state".to_string());
        }

        let vad = match vad_model.and_then(|path| CString::new(path).ok()) {
            // SAFETY: default VAD parameters are plain data and the path is a valid C string.
            Some(path) => unsafe {
                let mut params = sys::whisper_vad_default_context_params();
                params.n_threads = 1;
                params.use_gpu = false;
                sys::whisper_vad_init_from_file_with_params(path.as_ptr(), params)
            },
            None => ptr::null_mut(),
        };

        // SAFETY: default VAD parameters are plain data.
        let vad_params = unsafe { sys::whisper_vad_default_params() };

        Ok(Self {
            ctx,
            state,
            vad,
            vad_params,
            language,
            n_threads: n_threads.clamp(1, 2),
        })
    }

    /// Whether Silero VAD loaded; without it every clip is decoded.
    pub fn has_vad(&self) -> bool {
        !self.vad.is_null()
    }

    /// Transcribe 16 kHz mono PCM. Returns an empty string when the clip holds
    /// no speech, so callers can stay silent instead of acting on noise.
    pub fn transcribe(&mut self, samples: &[i16]) -> Result<String, String> {
        if samples.len() < MIN_SAMPLES {
            return Ok(String::new());
        }

        let mut audio: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

        if !self.vad.is_null() {
            match self.speech_window(&audio) {
                Ok(None) => return Ok(String::new()),
                Ok(Some((start, end))) if end > start => audio = audio[start..end].to_vec(),
                Ok(Some(_)) => return Ok(String::new()),
                Err(error) => return Err(error),
            }
        }
        if audio.len() < MIN_SAMPLES {
            return Ok(String::new());
        }

        // SAFETY: the default parameter block is plain data.
        let mut params = unsafe {
            sys::whisper_full_default_params(sys::whisper_sampling_strategy_WHISPER_SAMPLING_GREEDY)
        };
        params.n_threads = self.n_threads;
        params.translate = false;
        params.no_context = true;
        params.single_segment = true;
        params.no_timestamps = true;
        params.print_special = false;
        params.print_progress = false;
        params.print_realtime = false;
        params.print_timestamps = false;
        params.language = self.language.as_ptr();

        self.run_decode(&audio, params)
    }

    /// Decode one clip and join the text of its segments.
    fn run_decode(
        &mut self,
        audio: &[f32],
        params: sys::whisper_full_params,
    ) -> Result<String, String> {
        // SAFETY: the context, state, parameters and audio all outlive the call.
        let status = unsafe {
            sys::whisper_full_with_state(
                self.ctx,
                self.state,
                params,
                audio.as_ptr(),
                audio.len() as c_int,
            )
        };
        if status != 0 {
            return Err(format!("whisper.cpp decode failed (status {status})"));
        }

        // SAFETY: the state was just filled by `whisper_full_with_state`.
        let count = unsafe { sys::whisper_full_n_segments_from_state(self.state) };
        let mut text = String::new();
        for index in 0..count {
            // SAFETY: `index` is within the segment count read above.
            let piece = unsafe { sys::whisper_full_get_segment_text_from_state(self.state, index) };
            if piece.is_null() {
                continue;
            }
            // SAFETY: whisper.cpp returns a NUL-terminated string owned by the state.
            text.push_str(&unsafe { CStr::from_ptr(piece) }.to_string_lossy());
        }
        let text = text.trim().to_string();
        eprintln!("Luma STT: decode status={status} segments={count} text={text:?}");
        Ok(text)
    }

    /// Sample range Silero VAD classifies as speech, or `None` for silence.
    fn speech_window(&self, audio: &[f32]) -> Result<Option<(usize, usize)>, String> {
        // SAFETY: `vad` is non-null here and `audio` outlives the call.
        let segments = unsafe {
            sys::whisper_vad_segments_from_samples(
                self.vad,
                self.vad_params,
                audio.as_ptr(),
                audio.len() as c_int,
            )
        };
        if segments.is_null() {
            return Err("Silero VAD could not analyse the clip".to_string());
        }

        // SAFETY: `segments` is a live VAD result owned by this function.
        let count = unsafe { sys::whisper_vad_segments_n_segments(segments) };
        if count <= 0 {
            // SAFETY: `segments` was created above and is not used again.
            unsafe { sys::whisper_vad_free_segments(segments) };
            return Ok(None);
        }

        // SAFETY: the indices are inside `0..count`.
        let first = unsafe { sys::whisper_vad_segments_get_segment_t0(segments, 0) };
        let last = unsafe { sys::whisper_vad_segments_get_segment_t1(segments, count - 1) };
        // SAFETY: `segments` was created above and is not used again.
        unsafe { sys::whisper_vad_free_segments(segments) };

        // whisper.cpp reports VAD timestamps in centiseconds.
        let to_samples = |value: f32| ((value.max(0.0) / 100.0) * SAMPLE_RATE as f32) as usize;
        Ok(Some((
            to_samples(first).min(audio.len()),
            to_samples(last).min(audio.len()),
        )))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: each pointer is either null or was created by `load` and is
        // released exactly once, in reverse creation order.
        unsafe {
            if !self.vad.is_null() {
                sys::whisper_vad_free(self.vad);
            }
            if !self.state.is_null() {
                sys::whisper_free_state(self.state);
            }
            if !self.ctx.is_null() {
                sys::whisper_free(self.ctx);
            }
        }
    }
}
