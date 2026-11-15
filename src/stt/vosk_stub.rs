//! Desktop placeholder for the Vosk engine.
//!
//! Recognition runs on libvosk, which is only cross-built for the TV's ARM
//! target by `tools/build-vosk.sh`. Desktop builds keep the same API so the
//! Voice Assistant compiles unchanged, and every call says that recognition is
//! unavailable instead of pretending to listen.
#![allow(dead_code)]

/// No libvosk exists for this target, so loading always fails.
pub struct VoskEngine;

impl VoskEngine {
    pub fn load(_model_dir: &str) -> Result<Self, String> {
        Err(unavailable())
    }

    pub fn recognizer(&self, _sample_rate: f32) -> Result<Recognizer, String> {
        Err(unavailable())
    }
}

/// Placeholder recognizer. Nothing can construct it, because loading fails.
pub struct Recognizer;

impl Recognizer {
    pub fn accept(&mut self, _samples: &[i16]) -> Result<Option<String>, String> {
        Err(unavailable())
    }

    pub fn finish(&mut self) -> String {
        String::new()
    }
}

fn unavailable() -> String {
    "Vosk speech recognition is only built for the webOS ARM target".to_string()
}
