---
name: polish
description: Iteratively review and fix issues on the current branch until the review passes.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent, Edit, Write
---

# Polish

Review-fix loop on the current branch until it converges to LGTM. Require 3 consecutive LGTMs before declaring convergence (reviews are non-deterministic, especially for large changes). Maximum 25 iterations.

## Preflight

Check `git log main..HEAD --oneline`. If no commits ahead and no uncommitted changes, ask the user what they intended. Stage and commit any uncommitted changes before starting.

Count the commits ahead of main. This determines how fixes are committed in the loop:

- **Single commit**: use `/amend` to fold fixes into it each iteration.
- **Multiple commits**: create a single `polish: ...` commit on the first fix iteration, then `/amend` that same commit on subsequent iterations. This keeps all polish fixes in one commit rather than scattering them across iterations.

## State tracking

Track across iterations: **Fixed** (what was changed), **Dismissed** (why not fixed), **Pending user input** (needs a decision). When a re-raised issue was previously dismissed, skip it.

## Loop

1. **Review**: delegate to a fresh sub-agent with no prior context (so it reviews the code on its own merits, not anchored to previous findings): "Run `/review` on the current branch against main. Skip: PR description, Needs verification, Existing issues."
2. **Triage**: for each finding — skip if dismissed, fix or dismiss with rationale, collect ambiguous cases for user input. Watch for findings that reveal design gaps (wrong assumptions, flawed mental models) regardless of severity — escalate these to the user rather than auto-fixing, as they may require rethinking the approach.
3. **Fix**: apply fixes (🔴 > 🟠 > 🟡), run lint/format, then commit using the strategy determined in Preflight (amend for single-commit branches, new commit for multi-commit branches). Go to Step 1.
4. **Report**: `✅ LGTM after N iterations` or `⚠️ Stopped after N iterations`. List fixes by file, dismissed items, and remaining issues.
