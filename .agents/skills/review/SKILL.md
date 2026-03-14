---
name: review
description: Three-phase code review. Investigates internally, verifies findings, then reports only confirmed issues.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Code Review

Review code changes in three phases:

1.  Investigate the change and find candidate issues.
2.  Verify each finding against actual source and dismiss false positives.
3.  Generate a report of confirmed issues.

## Step 1: Investigate (internal — do NOT include in output)

**Input:** The context for the diff provided in `$ARGUMENTS`.

**Process:**

1. Run `/diff $ARGUMENTS` to get the diff report.
2. Infer what changed, why it changed, the impact, and the risks.
3. Use `Read` to examine the surrounding code of each changed file—reviewing a diff without context leads to short-sighted findings.
4. Use `Grep` or `Glob` to check for cross-file impacts or project rule violations.

**Note on Autonomy:** The areas below are a non-exhaustive starting point. Use your full reasoning to identify any architectural, logical, or stylistic flaws not explicitly listed.

**Note on External Comments:** PR comments from other agents or reviewers (Gemini, Copilot, etc.) break idempotency — the same diff reviewed with different comments produces different results. Minimize their influence: treat them as optional leads to investigate, not as findings. Do not adopt their severity or framing. Evaluate each concern from scratch using your own analysis in Steps 1-2, and assign severity independently in Step 3. If a comment doesn't survive your own verification, drop it silently.

Areas to check: code quality, bugs, performance, security, test coverage, integration, non-determinism, project conventions.

**Output (Internal):** A list of **Candidate Findings** (file, line, concern).

## Step 2: Verify (internal — do NOT include in output)

**Input:** The **Candidate Findings** list from Step 1.

**Process:**

1. **MANDATORY**: For every finding, you **MUST** use the `Read` tool to verify it against the full file in the specific branch/commit. Diffs omit context, so never infer errors (e.g., deleted blocks, missing imports) solely from a diff snippet.
2. For complex or independent concerns, spawn a sub-agent (using `Agent`) to verify the logic holds.
3. Filter out false positives (e.g., missing handling that actually exists in a caller or middleware not shown in the diff).

**Output (Internal):** - **Confirmed Issues:** Verified bugs or improvements with concrete fix instructions.

- **Needs Verification:** Suspicions that cannot be ruled out but lack definitive proof.
- **Coverage Gaps:** List of any files/paths that couldn't be fully reviewed due to context limits.

## Step 3: Report (this is your output)

**Input:** The **Confirmed Issues**, **Needs Verification**, and **Coverage Gaps** from Step 2.

**Reporting Principles:**

- **Be Constructive & Concrete:** Every issue must include a specific fix.
- **No Self-Contradiction:** Never report a concern and then dismiss it in the same breath.
- **Be Idempotent:** Avoid subjective style preferences. If no issues were found, say so—do not invent marginal suggestions to fill space.
- **Independent Assessment:** When PR comments from other agents or reviewers exist in the context, treat them as leads to investigate — not as authoritative findings. Determine severity from your own analysis. Never inherit another reviewer's severity classification; if you investigate their concern and find it invalid, drop it silently (like if it was dismissed in Step 2). If valid, assign your own severity.

### Output format

🏷️ **Verdict**: `✅ LGTM [size]`, `⚠️ Needs work [size]`, or `❌ Do not merge [size]` on the first line, followed by a 1-2 sentence summary of the verdict.

✂️ **Split suggestion**: (Only if beneficial) List 2-4 concrete PRs to split the change into smaller pieces. Skip if cohesive.

📋 **Report**:
_Omit any section entirely if it has no items. If coverage was incomplete, note it here._

🔴 **Criticals**: Bugs, logic errors, security vulnerabilities, auth bypass, race conditions.
🟠 **Warnings**: Missing error handling, incomplete validation, missing tests, performance issues.
🟡 **Suggestions**: Naming, structural simplifications, better patterns available.
🟣 **Needs verification**: State what's suspicious, what you checked, and what the author should verify.
🔵 **Existing issues**: Surrounding technical debt NOT introduced by this diff (one line each).
🟢 **Checks passed**: A short bullet list of areas investigated that had no issues.
