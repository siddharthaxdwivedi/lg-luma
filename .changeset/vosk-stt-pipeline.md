---
"lg-luma": minor
---

Replace the on-device whisper.cpp pipeline with Vosk. Whisper could not keep up with the TV's CPU:
a 0.93 s clip had still not finished decoding after 150 seconds, and every utterance paid for a full
30 s encoder pass. Vosk is a streaming recognizer, so the microphone is fed in as it arrives, the
utterance ends about a second after you stop talking, and nothing is decoded while the room is
quiet. One utterance is capped at 30 seconds. The noise-floor gate and its silence buffering are
gone; local command resolution and the cloud fallback are unchanged.
