//! Local Speech-to-Text using whisper.cpp.
//!
//! A dedicated thread owns the microphone. Opening the orb starts a 500 ms
//! noise-floor calibration, capture begins only when the level clears that
//! floor, and the utterance ends after 1.5 s of silence (see `gate.rs`). The
//! clip is then trimmed by Silero VAD and decoded in command mode on two
//! threads, so the TV never runs Whisper on silence and never saturates all
//! four cores.

mod commands;
mod engine;
mod gate;
mod grammar;

pub use commands::resolve;
use engine::Engine;
use gate::{Gate, GateEvent};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};

/// Commands sent to the STT thread.
pub enum SttCommand {
    StartListening,
    StopListening,
}

/// Events published by the STT thread.
#[derive(Debug, Clone)]
pub enum SttEvent {
    /// Speech was detected and is being captured.
    Listening,
    /// Capture finished; the clip is being decoded.
    Processing,
    /// Final transcript. Empty means the clip held no speech.
    Final(String),
    Error(String),
}

/// Upper bound on retained audio per utterance (~30 s at 16 kHz).
const MAX_SAMPLES: usize = 16_000 * 30;

/// Sampling rate whisper.cpp and Silero VAD expect.
const SAMPLE_RATE: f32 = 16_000.0;

pub struct SttConfig {
    pub model_path: String,
    pub vad_model_path: String,
    pub grammar_path: String,
    pub sample_rate: f32,
    pub chunk_frames: usize,
    /// Decoding threads. Capped at 2 so the TV keeps a core free.
    pub n_threads: i32,
    /// Set while TTS is speaking; capture pauses so the TV cannot hear itself.
    pub tts_speaking: Option<Arc<AtomicBool>>,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            model_path: asset_path("ggml-base.en.bin"),
            vad_model_path: asset_path("ggml-silero-v5.1.2.bin"),
            grammar_path: asset_path("commands.gbnf"),
            sample_rate: SAMPLE_RATE,
            chunk_frames: 1600,
            n_threads: 2,
            tts_speaking: None,
        }
    }
}

/// Resolve a bundled asset next to the running executable, falling back to the
/// installed application directory.
fn asset_path(name: &str) -> String {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        return dir.join("assets").join(name).to_string_lossy().into_owned();
    }
    format!("/media/developer/apps/usr/palm/applications/com.ecklf.luma/assets/{name}")
}

pub struct SttHandle {
    cmd_tx: Sender<SttCommand>,
    evt_rx: Receiver<SttEvent>,
    /// Mic RMS (f32 bits) shared with the Voice Orb.
    pub rms: Arc<AtomicI32>,
    _is_listening: Arc<AtomicBool>,
    _thread: thread::JoinHandle<()>,
}

impl SttHandle {
    pub fn spawn(config: SttConfig) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = bounded(4);
        let (evt_tx, evt_rx) = bounded(16);
        let rms = Arc::new(AtomicI32::new(0));
        let is_listening = Arc::new(AtomicBool::new(false));

        let rms_clone = rms.clone();
        let listening_clone = is_listening.clone();

        let thread = thread::Builder::new()
            .name("luma-stt".into())
            .spawn(move || {
                stt_thread(config, cmd_rx, evt_tx, rms_clone, listening_clone);
            })
            .map_err(|e| format!("failed to spawn STT thread: {e}"))?;

        Ok(Self {
            cmd_tx,
            evt_rx,
            rms,
            _is_listening: is_listening,
            _thread: thread,
        })
    }

    /// Open the orb: calibrate the room, then capture one utterance.
    pub fn start(&self) {
        let _ = self.cmd_tx.send(SttCommand::StartListening);
    }

    /// Close the orb early and transcribe whatever was captured.
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(SttCommand::StopListening);
    }

    pub fn try_recv(&self) -> Option<SttEvent> {
        self.evt_rx.try_recv().ok()
    }
}

fn stt_thread(
    config: SttConfig,
    cmd_rx: Receiver<SttCommand>,
    evt_tx: Sender<SttEvent>,
    rms: Arc<AtomicI32>,
    is_listening: Arc<AtomicBool>,
) {
    let grammar = load_grammar(&config.grammar_path);
    let mut engine = match Engine::load(
        &config.model_path,
        Some(&config.vad_model_path),
        grammar.as_ref(),
        config.n_threads,
    ) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = evt_tx.send(SttEvent::Error(error));
            return;
        }
    };
    if !engine.has_vad() {
        eprintln!("Luma STT: Silero VAD unavailable, decoding every clip");
    }

    let mut audio_source = match AudioSource::open(config.sample_rate) {
        Ok(source) => source,
        Err(error) => {
            let _ = evt_tx.send(SttEvent::Error(format!(
                "audio source open failed: {error}"
            )));
            return;
        }
    };

    let chunk_ms = (config.chunk_frames as f32 / config.sample_rate * 1000.0).max(1.0);
    let mut gate = Gate::new(chunk_ms);
    let mut samples: Vec<i16> = Vec::new();
    let mut buffer = vec![0i16; config.chunk_frames];
    let mut listening = false;

    loop {
        while let Ok(command) = cmd_rx.try_recv() {
            match command {
                SttCommand::StartListening => {
                    if !listening {
                        listening = true;
                        is_listening.store(true, Ordering::Relaxed);
                        samples.clear();
                        gate.reset();
                    }
                }
                SttCommand::StopListening => {
                    if listening {
                        listening = false;
                        is_listening.store(false, Ordering::Relaxed);
                        rms.store(0, Ordering::Relaxed);
                        gate.reset();
                        finish(&mut engine, &samples, &evt_tx);
                        samples.clear();
                    }
                }
            }
        }

        if !listening {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Echo guard: never listen while the TV is speaking.
        if config
            .tts_speaking
            .as_ref()
            .is_some_and(|speaking| speaking.load(Ordering::Relaxed))
        {
            thread::sleep(Duration::from_millis(20));
            continue;
        }

        let frames = match audio_source.read_chunk(&mut buffer) {
            Ok(frames) => frames,
            Err(error) => {
                let _ = evt_tx.send(SttEvent::Error(format!("audio read error: {error}")));
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if frames == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        let chunk = &buffer[..frames];
        let sum: f32 = chunk
            .iter()
            .map(|&sample| (f32::from(sample) / 32768.0).powi(2))
            .sum();
        let level = (sum / frames as f32).sqrt().min(1.0);
        rms.store(level.to_bits() as i32, Ordering::Relaxed);

        match gate.push(level, samples.len() >= MAX_SAMPLES) {
            GateEvent::Ignore => {}
            GateEvent::Start => {
                samples.clear();
                push_samples(&mut samples, chunk);
                let _ = evt_tx.send(SttEvent::Listening);
            }
            GateEvent::Keep => push_samples(&mut samples, chunk),
            GateEvent::Discard => samples.clear(),
            GateEvent::End => {
                push_samples(&mut samples, chunk);
                listening = false;
                is_listening.store(false, Ordering::Relaxed);
                rms.store(0, Ordering::Relaxed);
                gate.reset();
                let _ = evt_tx.send(SttEvent::Processing);
                finish(&mut engine, &samples, &evt_tx);
                samples.clear();
            }
        }
    }
}

fn load_grammar(path: &str) -> Option<grammar::Grammar> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("Luma STT: no command grammar at {path}: {error}");
            return None;
        }
    };
    match grammar::parse(&source) {
        Ok(grammar) => Some(grammar),
        Err(error) => {
            eprintln!("Luma STT: ignoring command grammar {path}: {error}");
            None
        }
    }
}

fn push_samples(samples: &mut Vec<i16>, chunk: &[i16]) {
    if samples.len() >= MAX_SAMPLES {
        return;
    }
    let take = chunk.len().min(MAX_SAMPLES - samples.len());
    samples.extend_from_slice(&chunk[..take]);
}

fn finish(engine: &mut Engine, samples: &[i16], evt_tx: &Sender<SttEvent>) {
    match engine.transcribe(samples) {
        Ok(text) => {
            let _ = evt_tx.send(SttEvent::Final(text));
        }
        Err(error) => {
            let _ = evt_tx.send(SttEvent::Error(error));
        }
    }
}

enum AudioSource {
    VoiceInput(std::os::unix::net::UnixStream),
    Alsa(alsa::pcm::PCM),
}

impl AudioSource {
    fn open(sample_rate: f32) -> Result<Self, String> {
        const VOICE_INPUT_SOCK: &str = "/tmp/voiceinput_remote1.soc";
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(VOICE_INPUT_SOCK) {
            stream.set_nonblocking(true).ok();
            return Ok(Self::VoiceInput(stream));
        }

        #[cfg(target_os = "linux")]
        {
            use alsa::{
                Direction, ValueOr,
                pcm::{Access, Format, HwParams, PCM},
            };
            let pcm = PCM::new("default", Direction::Capture, false)
                .map_err(|e| format!("ALSA open failed: {e}"))?;
            let hwp = HwParams::any(&pcm).map_err(|e| format!("HwParams any failed: {e}"))?;
            hwp.set_access(Access::RWInterleaved)
                .map_err(|e| format!("set_access failed: {e}"))?;
            hwp.set_format(Format::s16())
                .map_err(|e| format!("set_format failed: {e}"))?;
            hwp.set_channels(1)
                .map_err(|e| format!("set_channels failed: {e}"))?;
            let rate = sample_rate as u32;
            hwp.set_rate_near(rate, ValueOr::Nearest)
                .map_err(|e| format!("set_rate failed: {e}"))?;
            pcm.hw_params(&hwp)
                .map_err(|e| format!("hw_params failed: {e}"))?;
            pcm.prepare().map_err(|e| format!("prepare failed: {e}"))?;
            pcm.start().map_err(|e| format!("start failed: {e}"))?;
            drop(hwp);
            Ok(Self::Alsa(pcm))
        }

        #[cfg(not(target_os = "linux"))]
        return Err("no audio source available".to_string());
    }

    fn read_chunk(&mut self, buffer: &mut [i16]) -> Result<usize, String> {
        match self {
            Self::VoiceInput(stream) => {
                use std::io::Read;
                let mut bytes = vec![0u8; buffer.len() * 2];
                match stream.read(&mut bytes) {
                    Ok(n) if n > 0 => {
                        let frames = n / 2;
                        for i in 0..frames {
                            buffer[i] = i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
                        }
                        Ok(frames)
                    }
                    Ok(_) => Ok(0),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
                    Err(e) => Err(e.to_string()),
                }
            }
            Self::Alsa(pcm) => {
                // ALSA 0.12: use io_i16().readi() - returns Result<usize>
                let io = pcm
                    .io_i16()
                    .map_err(|e| format!("ALSA io_i16 failed: {e}"))?;
                let frames = io
                    .readi(buffer)
                    .map_err(|e| format!("ALSA readi failed: {e}"))?;
                Ok(frames)
            }
        }
    }
}
