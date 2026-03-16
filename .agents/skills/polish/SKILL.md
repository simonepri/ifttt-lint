---
name: polish
description: Iteratively review and fix issues on the current branch until the review passes.
---

# Polish

Review-fix loop on the current branch until it converges to LGTM. Require 3 consecutive LGTMs before declaring convergence (reviews are non-deterministic, especially for large changes). Maximum 50 iterations.

## Preflight

Check `git log main..HEAD --oneline`. If no commits ahead and no uncommitted changes, ask the user what they intended. Stage and commit any uncommitted changes before starting.

## State tracking

Track across iterations: **Fixed** (what was changed), **Dismissed** (why not fixed), **Pending user input** (needs a decision). When a re-raised issue was previously dismissed, skip it.

## Loop

1. **Review**: delegate to a fresh sub-agent with no prior context (so it reviews the code on its own merits, not anchored to previous findings): "Run `/review` on the current branch against main. Skip: PR description, Needs verification, Existing issues."
2. **Triage**: for each finding — skip if dismissed, fix or dismiss with rationale, collect ambiguous cases for user input. Watch for findings that reveal design gaps (wrong assumptions, flawed mental models) regardless of severity — escalate these to the user rather than auto-fixing, as they may require rethinking the approach.
3. **Fix**: apply fixes (🔴 > 🟠 > 🟡), run lint/format, amend into the last commit (`git add -A && git commit --amend --no-edit`), go to Step 1.
4. **Report**: `✅ LGTM after N iterations` or `⚠️ Stopped after N iterations`. List fixes by file, dismissed items, and remaining issues.
