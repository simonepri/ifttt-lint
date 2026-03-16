---
name: commit
description: Stage changes and create a commit with a conventional commit message.
---

# Commit

1. **Assess**: run `git status` and `git diff` to see changes.
2. **Stage**: relevant files only. If $ARGUMENTS contains file paths, stage only those. Never stage sensitive files (.env, credentials) without confirmation.
3. **Message**: analyze the staged diff. Write a conventional commit message:
   - Format: `type(scope): subject` — types: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `ci`, `perf`
   - Breaking changes: append `!` (e.g., `fix!: remove deprecated endpoint`)
   - Subject: under 72 chars, imperative mood. Must reference _what_ changed specifically — generic subjects like "initial commit", "fix bug", "update code" are never acceptable.
   - Body: use the reasoning framework (Problem/Solution) always
   - One logical change per commit. If you need "and" in the subject, split it.
4. **Commit**: `git commit -m "<message>"` — do not add Co-Authored-By or other trailers.
