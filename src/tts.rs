//! Local Text-to-Speech using espeak-ng.
//!
//! Runs on a dedicated background thread. Accepts text to speak, synthesizes
//! it via espeak-ng, and computes real-time RMS for the Voice Orb's
//! dual-reactivity pulse.

#[cfg(feature = "voice-assistant")]
use std::path::PathBuf;
#[cfg(feature = "voice-assistant")]
use std::sync::Arc;
#[cfg(feature = "voice-assistant")]
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
#[cfg(feature = "voice-assistant")]
use std::thread;
#[cfg(feature = "voice-assistant")]
use std::time::Duration;

#[cfg(feature = "voice-assistant")]
use crossbeam_channel::{Receiver, Sender, bounded};
#[cfg(feature = "voice-assistant")]
use espeak_ng::{EspeakNg, Parameter};

/// Commands sent to the TTS thread.
pub enum TtsCommand {
    Speak(String),
    Stop,
}

/// Events published by the TTS thread.
#[derive(Debug, Clone)]
pub enum TtsEvent {
    Started,
    Finished,
    Error,
}

/// Handle to the TTS background thread.
pub struct TtsHandle {
    cmd_tx: Sender<TtsCommand>,
    _evt_rx: Receiver<TtsEvent>,
    /// Shared RMS level for the Voice Orb (0.0-1.0, stored as f32 bits).
    _rms: Arc<AtomicI32>,
    /// Whether TTS is currently speaking.
    is_speaking: Arc<AtomicBool>,
    _thread: thread::JoinHandle<()>,
}

impl TtsHandle {
    /// Spawn the TTS thread and return a handle.
    pub fn spawn(tts_rms_atomic: Arc<AtomicI32>) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = bounded(4);
        let (evt_tx, evt_rx) = bounded(16);
        let rms = Arc::new(AtomicI32::new(0));
        let is_speaking = Arc::new(AtomicBool::new(false));

        let rms_clone = rms.clone();
        let speaking_clone = is_speaking.clone();
        let tts_rms_atomic = tts_rms_atomic.clone();

        let thread = thread::Builder::new()
            .name("luma-tts".into())
            .spawn(move || {
                tts_thread(cmd_rx, evt_tx, rms_clone, speaking_clone, tts_rms_atomic);
            })
            .map_err(|e| format!("failed to spawn TTS thread: {e}"))?;

        Ok(Self {
            cmd_tx,
            _evt_rx: evt_rx,
            _rms: rms,
            is_speaking,
            _thread: thread,
        })
    }

    /// Speak the given text.
    pub fn speak(&self, text: String) {
        let _ = self.cmd_tx.send(TtsCommand::Speak(text));
    }

    /// Stop current speech.
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(TtsCommand::Stop);
    }

    /// Whether the TTS thread is currently speaking. STT reads this to avoid
    /// transcribing the TV's own voice.
    pub fn speaking_flag(&self) -> Arc<AtomicBool> {
        self.is_speaking.clone()
    }

    /// Try to receive the next TTS event (non-blocking).
    pub fn _try_recv(&self) -> Option<TtsEvent> {
        self._evt_rx.try_recv().ok()
    }
}

/// Main TTS thread loop.
fn tts_thread(
    cmd_rx: Receiver<TtsCommand>,
    evt_tx: Sender<TtsEvent>,
    rms: Arc<AtomicI32>,
    is_speaking: Arc<AtomicBool>,
    tts_rms_atomic: Arc<AtomicI32>,
) {
    // espeak-ng reads dictionaries and phoneme tables from disk. webOS ships no
    // system copy, so extract the bundled English data into a writable
    // directory once, then initialise from there.
    let data_dir = espeak_data_dir();
    let mut espeak = match EspeakNg::with_data_dir("en", &data_dir) {
        Ok(e) => e,
        Err(error) => {
            eprintln!("Voice Assistant TTS data: {error}");
            let _ = evt_tx.send(TtsEvent::Error);
            return;
        }
    };

    // Set speech rate (words per minute).
    espeak.set_parameter(Parameter::Rate, 175);

    loop {
        // Handle commands (non-blocking).
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                TtsCommand::Speak(text) => {
                    is_speaking.store(true, Ordering::Relaxed);
                    let _ = evt_tx.send(TtsEvent::Started);
                    match espeak.synth(&text) {
                        Ok((samples, rate)) => {
                            play_pcm(&samples, rate, &tts_rms_atomic);
                        }
                        Err(_e) => {
                            let _ = evt_tx.send(TtsEvent::Error);
                        }
                    }
                    is_speaking.store(false, Ordering::Relaxed);
                    rms.store(0, Ordering::Relaxed);
                    tts_rms_atomic.store(0, Ordering::Relaxed);
                    let _ = evt_tx.send(TtsEvent::Finished);
                }
                TtsCommand::Stop => {
                    is_speaking.store(false, Ordering::Relaxed);
                    rms.store(0, Ordering::Relaxed);
                    tts_rms_atomic.store(0, Ordering::Relaxed);
                }
            }
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Writable directory holding the bundled espeak-ng data. Extracted once; the
/// crate ships no data and webOS has no system espeak-ng-data.
fn espeak_data_dir() -> PathBuf {
    let candidates = [
        PathBuf::from("/var/lib/luma-home/espeak-ng-data"),
        std::env::temp_dir().join("luma-espeak-ng-data"),
    ];
    for directory in candidates {
        if directory.join("en_dict").exists() {
            return directory;
        }
        if std::fs::create_dir_all(&directory).is_ok()
            && espeak_ng::install_bundled_language(&directory, "en").is_ok()
        {
            return directory;
        }
    }
    std::env::temp_dir().join("luma-espeak-ng-data")
}

/// Play synthesized 16-bit mono PCM through the TV's default output. espeak-ng
/// is a synthesis-only library, so the samples must be written to ALSA. RMS is
/// published per chunk so the Voice Orb pulses with the actual speech.
fn play_pcm(samples: &[i16], rate: u32, tts_rms_atomic: &AtomicI32) {
    use alsa::{
        Direction, ValueOr,
        pcm::{Access, Format, HwParams, PCM},
    };
    let pcm = match PCM::new("default", Direction::Playback, false) {
        Ok(pcm) => pcm,
        Err(error) => {
            eprintln!("Voice Assistant TTS playback: {error}");
            return;
        }
    };
    let hwp = match HwParams::any(&pcm) {
        Ok(hwp) => hwp,
        Err(error) => {
            eprintln!("Voice Assistant TTS playback params: {error}");
            return;
        }
    };
    let configured = (|| -> Result<(), alsa::Error> {
        hwp.set_access(Access::RWInterleaved)?;
        hwp.set_format(Format::s16())?;
        hwp.set_channels(1)?;
        hwp.set_rate_near(rate, ValueOr::Nearest)?;
        pcm.hw_params(&hwp)?;
        pcm.prepare()?;
        Ok(())
    })();
    if let Err(error) = configured {
        eprintln!("Voice Assistant TTS playback setup: {error}");
        return;
    }
    let io = match pcm.io_i16() {
        Ok(io) => io,
        Err(error) => {
            eprintln!("Voice Assistant TTS playback io: {error}");
            return;
        }
    };
    let mut written = 0usize;
    for chunk in samples.chunks(1024) {
        tts_rms_atomic.store(compute_rms(chunk).to_bits() as i32, Ordering::Relaxed);
        if io.writei(chunk).is_err() {
            eprintln!("VOICE tts sink write failed after={written} frames");
            break;
        }
        written += chunk.len();
    }
    eprintln!("VOICE tts sink played frames={written} rate={rate}");
    let _ = pcm.drain();
    tts_rms_atomic.store(0, Ordering::Relaxed);
}

/// Compute the RMS of 16-bit mono PCM samples, normalized to 0.0-1.0.
fn compute_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum();
    (sum_sq / samples.len() as f32).sqrt().min(1.0)
}
