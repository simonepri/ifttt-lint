---
name: review
description: Two-phase code review. Investigates internally, then reports only confirmed issues.
---

# Code Review

Review code changes in two phases: investigate internally, then report only confirmed issues.

## Step 1: Understand the change

Run `/diff $ARGUMENTS` to get the diff (this also fetches the PR description or commit messages for context). Read the surrounding code of each changed file to understand the intent and design — reviewing a diff without context leads to short-sighted findings. Classify the change size:

| Size | Lines   | Files | Note                     |
| ---- | ------- | ----- | ------------------------ |
| XS   | < 50    | 1-2   | Trivial                  |
| S    | 50-200  | 3-5   | Ideal                    |
| M    | 200-400 | 6-10  | Standard                 |
| L    | 400-1k  | 11-20 | High cognitive load      |
| XL   | 1k+     | 20+   | Should probably be split |

## Step 2: Investigate (internal — do NOT include in output)

For each area below, trace the code and determine whether a concern is real or a false alarm. Dismiss false alarms silently. Read surrounding code for context.

Areas: code quality, bugs, performance, security, test coverage, integration, project conventions.

Rules:

- **Never report a concern and then dismiss it.** If unsure, investigate deeper or put it in Needs verification.
- **Be constructive.** Every issue includes a concrete fix.
- **Be idempotent.** Same diff = same findings. No subjective style preferences.

## Step 3: Report

Only report issues confirmed in Step 2.

**Verdict** (first line):

- `✅ LGTM [size]` — no issues or only suggestions
- `⚠️ Needs work [size]` — warnings worth addressing
- `❌ Do not merge [size]` — critical issues

**Split suggestion** (only if the change touches multiple independent concerns).

**Issues** (omit empty sections):

- 🔴 **Critical**: bugs, security, data loss
- 🟠 **Warning**: missing validation, tests, error handling
- 🟡 **Suggestion**: naming, structure, patterns

Each issue: file path + line range, what's wrong, concrete fix.

🟣 **Needs verification**: concerns you couldn't confirm or rule out.
🔵 **Existing issues**: problems in surrounding code, not introduced by this diff. One line each.
🟢 **Checks passed**: areas investigated with no issues.
