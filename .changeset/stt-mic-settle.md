---
"lg-luma": patch
---

Ignore the microphone's power-on transient while calibrating the voice gate. USB capture devices decay from a loud spike back to room noise over about a second; sampling that spike as the noise floor pushed the speech threshold so high that the orb could never hear a spoken command.
