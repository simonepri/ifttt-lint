---
name: diff
description: Fetch and display diffs for various scopes (PR, commit, branch, staged, working tree). Used by other skills that need to analyze changes.
---

# Diff

Fetch a diff based on the input scope.

## Scope detection

$ARGUMENTS determines the scope. Auto-detect the type:

| Input                         | Diff command               |
| ----------------------------- | -------------------------- |
| A PR number (e.g., `123`)     | `gh pr diff 123`           |
| A commit SHA (7-40 hex chars) | `git diff {sha}~1 {sha}`   |
| A commit range (`a..b`)       | `git diff a b`             |
| A branch name                 | `git diff main...{branch}` |
| `staged` or `pending`         | `git diff --staged`        |
| `current` or empty            | `git diff HEAD`            |

If empty and the working tree is clean, fall back to the current branch vs main.

## Output

Write the diff to a temp file and read it with the Read tool — diffs frequently exceed inline output limits.

If reviewing a PR, also read the PR description. If reviewing a commit, also read the commit messages.
