---
name: amend
description: Amend the last commit with current changes and/or an updated message.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Amend

Amend the last commit. Assumes the existing commit message is correct — only update it to reflect the delta.

1. **Safety**: check if the last commit exists on the remote. If it does, warn that amending requires a force push and ask for confirmation.
2. **Stage** relevant files. If $ARGUMENTS contains file paths, stage only those.
3. **Message**: read the existing commit message and the newly staged diff (`git diff --staged`). Update the message only if the delta changes the "what" or "why" — don't rewrite from scratch. Use the same body format as `/commit` (Problem + Solution with `##` headings).
4. **Amend**: create a unique temp file with `mktemp /tmp/commit-msg.XXXXXX.txt`, write the message to it, and run `git commit --amend -F <temp-file>` — do not add Co-Authored-By or other trailers.
