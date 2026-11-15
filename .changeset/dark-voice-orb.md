---
"lg-luma": minor
---

Redesign the voice orb as a dark polished sphere with a drifting iridescent seam, and sit it low on
the screen instead of at the centre. The sphere is drawn premultiplied-over, so it shades the
wallpaper behind it instead of glowing additively over it.

Whisper's command grammar now falls back to an unconstrained decode when it admits no tokens, so a
phrase the grammar does not cover is still transcribed instead of being dropped silently.

Enabling Luma as Home also stops LG's voice assistant units, so LG's own overlay can no longer
cover Luma's orb when the remote's voice key is pressed.
