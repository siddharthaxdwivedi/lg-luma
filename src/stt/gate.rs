//! Endpointing for one spoken command.
//!
//! The microphone opens with 500 ms of room-noise calibration, capture starts
//! only when the level jumps clear of that noise floor, and the utterance ends
//! after 1.5 s of continuous silence. Keeping this as a pure state machine
//! makes the thresholds testable without a microphone or a TV.

/// Chunks of room noise sampled before listening for speech (500 ms at 100 ms).
const CALIBRATION_CHUNKS: usize = 5;
/// Ignore bursts shorter than this (300 ms at 100 ms).
const MIN_SPEECH_CHUNKS: usize = 3;
/// Absolute RMS margin above the noise floor that counts as speech.
const SPEECH_MARGIN: f32 = 0.010;
/// Relative RMS margin above the noise floor that counts as speech.
const SPEECH_RATIO: f32 = 2.5;
/// Relative RMS margin below the noise floor that counts as silence.
const SILENCE_RATIO: f32 = 1.6;
/// Absolute RMS margin below the noise floor that counts as silence.
const SILENCE_MARGIN: f32 = 0.004;
/// Continuous silence that ends the utterance.
const ENDPOINT_MS: f32 = 1_500.0;

/// What the capture loop should do with the chunk it just measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateEvent {
    /// Calibrating or waiting for speech; drop the chunk.
    Ignore,
    /// Speech just started; start a fresh buffer with this chunk.
    Start,
    /// Speech is continuing; append the chunk.
    Keep,
    /// A burst too short to be a command; drop the whole buffer.
    Discard,
    /// The utterance finished; append the chunk and transcribe.
    End,
}

/// Noise floor plus speech/silence hysteresis for a single utterance.
pub struct Gate {
    endpoint_chunks: usize,
    calibrating: usize,
    calibration_total: f32,
    floor: f32,
    capturing: bool,
    speech_chunks: usize,
    silent_chunks: usize,
}

impl Gate {
    /// `chunk_ms` is the duration of one capture chunk.
    pub fn new(chunk_ms: f32) -> Self {
        let chunk_ms = chunk_ms.max(1.0);
        let mut gate = Self {
            endpoint_chunks: (ENDPOINT_MS / chunk_ms).ceil() as usize,
            calibrating: 0,
            calibration_total: 0.0,
            floor: 0.0,
            capturing: false,
            speech_chunks: 0,
            silent_chunks: 0,
        };
        gate.reset();
        gate
    }

    /// Begin a new utterance: re-calibrate and forget the previous capture.
    pub fn reset(&mut self) {
        self.calibrating = CALIBRATION_CHUNKS;
        self.calibration_total = 0.0;
        self.floor = 0.0;
        self.capturing = false;
        self.speech_chunks = 0;
        self.silent_chunks = 0;
    }

    /// Measured room noise, or 0 before calibration finishes.
    #[cfg(test)]
    pub fn floor(&self) -> f32 {
        self.floor
    }

    /// Whether the gate is holding an utterance.
    #[cfg(test)]
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    /// Feed one chunk's RMS level. `full` forces the end once the buffer is at
    /// its sample limit.
    pub fn push(&mut self, level: f32, full: bool) -> GateEvent {
        if self.calibrating > 0 {
            self.calibration_total += level;
            self.calibrating -= 1;
            if self.calibrating == 0 {
                self.floor = self.calibration_total / CALIBRATION_CHUNKS as f32;
            }
            return GateEvent::Ignore;
        }

        let speech = level > self.floor * SPEECH_RATIO + SPEECH_MARGIN;

        if !self.capturing {
            if speech {
                self.capturing = true;
                self.speech_chunks = 1;
                self.silent_chunks = 0;
                return GateEvent::Start;
            }
            return GateEvent::Ignore;
        }

        if speech {
            self.speech_chunks += 1;
            self.silent_chunks = 0;
        } else if level <= self.floor * SILENCE_RATIO + SILENCE_MARGIN {
            self.silent_chunks += 1;
        } else {
            self.silent_chunks = 0;
        }

        if self.silent_chunks >= self.endpoint_chunks || full {
            if self.speech_chunks < MIN_SPEECH_CHUNKS {
                self.reset();
                return GateEvent::Discard;
            }
            self.capturing = false;
            return GateEvent::End;
        }

        GateEvent::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibrated(noise: f32) -> Gate {
        let mut gate = Gate::new(100.0);
        for _ in 0..CALIBRATION_CHUNKS {
            assert_eq!(gate.push(noise, false), GateEvent::Ignore);
        }
        assert!((gate.floor() - noise).abs() < 1e-6);
        gate
    }

    #[test]
    fn ignores_room_noise_after_calibration() {
        let mut gate = calibrated(0.002);
        for _ in 0..50 {
            assert_eq!(gate.push(0.0025, false), GateEvent::Ignore);
        }
        assert!(!gate.capturing());
    }

    #[test]
    fn starts_on_speech_and_ends_after_1_5_seconds_of_silence() {
        let mut gate = calibrated(0.002);
        assert_eq!(gate.push(0.05, false), GateEvent::Start);
        for _ in 0..5 {
            assert_eq!(gate.push(0.05, false), GateEvent::Keep);
        }
        for _ in 0..14 {
            assert_eq!(gate.push(0.002, false), GateEvent::Keep);
        }
        assert_eq!(gate.push(0.002, false), GateEvent::End);
        assert!(!gate.capturing());
    }

    #[test]
    fn a_short_burst_is_discarded() {
        let mut gate = calibrated(0.002);
        assert_eq!(gate.push(0.05, false), GateEvent::Start);
        for _ in 0..14 {
            assert_eq!(gate.push(0.001, false), GateEvent::Keep);
        }
        assert_eq!(gate.push(0.001, false), GateEvent::Discard);
        assert!(!gate.capturing());
    }

    #[test]
    fn a_loud_burst_inside_silence_does_not_end_the_utterance() {
        let mut gate = calibrated(0.002);
        assert_eq!(gate.push(0.05, false), GateEvent::Start);
        for _ in 0..10 {
            assert_eq!(gate.push(0.002, false), GateEvent::Keep);
        }
        assert_eq!(gate.push(0.05, false), GateEvent::Keep);
        for _ in 0..10 {
            assert_eq!(gate.push(0.002, false), GateEvent::Keep);
        }
    }

    #[test]
    fn a_full_buffer_ends_the_utterance() {
        let mut gate = calibrated(0.002);
        assert_eq!(gate.push(0.05, false), GateEvent::Start);
        assert_eq!(gate.push(0.05, false), GateEvent::Keep);
        assert_eq!(gate.push(0.05, false), GateEvent::Keep);
        assert_eq!(gate.push(0.05, true), GateEvent::End);
    }

    #[test]
    fn reset_recalibrates() {
        let mut gate = calibrated(0.002);
        assert_eq!(gate.push(0.05, false), GateEvent::Start);
        gate.reset();
        assert!(!gate.capturing());
        assert_eq!(gate.push(0.002, false), GateEvent::Ignore);
        assert_eq!(gate.floor(), 0.0);
    }
}
