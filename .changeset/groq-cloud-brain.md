---
"lg-luma": patch
---

Switch the cloud assistant from NVIDIA NIM to Groq. The request is the same
OpenAI-compatible chat completion, so only the endpoint, the model
(`qwen/qwen3.8-27b`) and the key file (`/var/lib/luma-home/.groq_key`) change.
Local command resolution, web search, conversation memory and TTS are untouched.
