---
name: amend
description: Amend the last commit with current changes and/or an updated message.
---

# Amend

Amend the last commit. Assumes the existing commit message is correct — only update it to reflect the delta.

1. **Safety**: check if the last commit exists on the remote. If it does, warn that amending requires a force push and ask for confirmation.
2. **Stage** relevant files. If $ARGUMENTS contains file paths, stage only those.
3. **Message**: read the existing commit message and the newly staged diff (`git diff --staged`). Update the message only if the delta changes the "what" or "why" — don't rewrite from scratch.
4. **Amend**: `git commit --amend -m "<message>"` — do not add Co-Authored-By or other trailers.
