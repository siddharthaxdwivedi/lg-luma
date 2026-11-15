---
"lg-luma": patch
---

Transcribe speech unconstrained instead of forcing it through the command grammar. The grammar
turned any clip into a command, so a question like "who owns YouTube" could launch YouTube instead
of being answered. Command verbs are still resolved locally; everything else now goes to the cloud
assistant.
