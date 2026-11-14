//! Minimal FFI layer over whisper.cpp for command-mode transcription.
//!
//! `whisper-rs`'s safe wrapper cannot express command mode: its
//! `FullParams::set_grammar` casts a flat element slice into
//! `const whisper_grammar_element **`, which whisper.cpp dereferences as one
//! pointer per rule, so any grammar would crash the TV. The raw bindings are
//! used here so the grammar, Silero VAD and thread limits stay explicit.

use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;

use whisper_rs_sys as sys;

use super::grammar::{Element, ElementKind, Grammar};

/// whisper.cpp works at 16 kHz mono.
pub const SAMPLE_RATE: usize = 16_000;

/// Shortest clip worth decoding (~0.3 s).
const MIN_SAMPLES: usize = SAMPLE_RATE / 3;

/// A loaded whisper.cpp context plus the grammar it should decode with.
pub struct Engine {
    ctx: *mut sys::whisper_context,
    state: *mut sys::whisper_state,
    vad: *mut sys::whisper_vad_context,
    vad_params: sys::whisper_vad_params,
    rules: Vec<Vec<sys::whisper_grammar_element>>,
    language: CString,
    n_threads: c_int,
}

impl Engine {
    /// Load a GGML model, the optional Silero VAD model, and the optional
    /// command grammar. `n_threads` is capped so the TV keeps a core free.
    pub fn load(
        model: &str,
        vad_model: Option<&str>,
        grammar: Option<&Grammar>,
        n_threads: c_int,
    ) -> Result<Self, String> {
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
            rules: grammar.map(compile).unwrap_or_default(),
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

        // The pointer array must outlive `whisper_full_with_state`.
        let mut rules: Vec<*const sys::whisper_grammar_element> =
            self.rules.iter().map(|rule| rule.as_ptr()).collect();
        if !rules.is_empty() {
            params.grammar_rules = rules.as_mut_ptr();
            params.n_grammar_rules = rules.len();
            params.i_start_rule = 0;
        }

        // SAFETY: the context, state, parameters, audio and grammar all outlive the call.
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
        Ok(text.trim().to_string())
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

/// Convert parsed rules into the flat, `END`-terminated element lists that
/// whisper.cpp's grammar engine expects.
fn compile(grammar: &Grammar) -> Vec<Vec<sys::whisper_grammar_element>> {
    grammar
        .rules
        .iter()
        .map(|rule| {
            let mut converted: Vec<sys::whisper_grammar_element> =
                rule.iter().map(to_c_type).collect();
            if converted.last().map(|element| element.type_)
                != Some(sys::whisper_gretype_WHISPER_GRETYPE_END)
            {
                converted.push(sys::whisper_grammar_element {
                    type_: sys::whisper_gretype_WHISPER_GRETYPE_END,
                    value: 0,
                });
            }
            converted
        })
        .collect()
}

fn to_c_type(element: &Element) -> sys::whisper_grammar_element {
    let type_ = match element.kind {
        ElementKind::End => sys::whisper_gretype_WHISPER_GRETYPE_END,
        ElementKind::Alt => sys::whisper_gretype_WHISPER_GRETYPE_ALT,
        ElementKind::RuleRef => sys::whisper_gretype_WHISPER_GRETYPE_RULE_REF,
        ElementKind::Char => sys::whisper_gretype_WHISPER_GRETYPE_CHAR,
        ElementKind::CharNot => sys::whisper_gretype_WHISPER_GRETYPE_CHAR_NOT,
        ElementKind::CharRangeUpper => sys::whisper_gretype_WHISPER_GRETYPE_CHAR_RNG_UPPER,
        ElementKind::CharAlt => sys::whisper_gretype_WHISPER_GRETYPE_CHAR_ALT,
    };
    sys::whisper_grammar_element {
        type_,
        value: element.value,
    }
}
