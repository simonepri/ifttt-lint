---
name: commit
description: Stage changes and create a commit with a conventional commit message.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Commit

1. **Assess**: run `git status` and `git diff` to see changes.
2. **Stage**: relevant files only. If $ARGUMENTS contains file paths, stage only those. If empty, stage all changed files or ask the user. Never stage sensitive files (.env, credentials) without confirmation.
3. **Message**: analyze the staged diff. Write a conventional commit message:
   - Format: `type(scope): subject` — types: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`, `ci`, `perf`
   - Breaking changes: append `!` (e.g., `fix!: remove deprecated endpoint`)
   - Subject: under 72 chars, imperative mood. Must reference _what_ changed specifically — generic subjects like "initial commit", "fix bug", "update code" are never acceptable.
   - Body: always use the reasoning framework with `##` markdown headings. Required: Problem + Solution. Optional (include only when they add non-obvious context): Goal, Background, Alternatives.

     ```
     ## Problem
     <what was wrong or missing>

     ## Goal (optional)
     <what this change aims to achieve, if not obvious>

     ## Background (optional)
     <context needed to understand why, if non-obvious>

     ## Solution
     <what was done and why this approach>

     ## Alternatives considered (optional)
     <what else was evaluated and why not>
     ```

   - One logical change per commit. If you need "and" in the subject, split it.

4. **Commit**: create a unique temp file with `mktemp /tmp/commit-msg.XXXXXX.txt`, write the message to it, and run `git commit -F <temp-file>` — do not add Co-Authored-By or other trailers.
