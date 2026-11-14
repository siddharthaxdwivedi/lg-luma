---
"lg-luma": minor
---

Replace the Vosk speech-to-text engine with an on-device whisper.cpp pipeline. Speech is now
endpointed on the TV: the orb calibrates the room noise floor for 500 ms, captures only speech,
and stops after 1.5 s of silence. Silero VAD trims silence before decoding, decoding is capped at
two threads, and a command-mode grammar biases Whisper toward TV phrases so matched app launches
run locally without a network round trip. Capture pauses while TTS speaks to avoid feedback.
