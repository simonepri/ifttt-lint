---
name: diff
description: Fetch and display diffs for various scopes (PR, commit, branch, staged, working tree). Used by other skills that need to analyze changes.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Diff

Fetch a diff based on the input scope.

## Scope detection

$ARGUMENTS determines the scope. Auto-detect the type:

| Input                         | Diff command                                                                                                                 |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| A PR number (e.g., `123`)     | `gh pr diff 123`                                                                                                             |
| A commit SHA (7-40 hex chars) | `git diff {sha}~1 {sha}`                                                                                                     |
| A commit range (`a..b`)       | `git diff a b`                                                                                                               |
| A branch name                 | `git diff {base}...{branch}` (detect base: `gh pr view --json baseRefName -q .baseRefName 2>/dev/null`, fall back to `main`) |
| `staged` or `pending`         | `git diff --staged`                                                                                                          |
| `current` or empty            | `git diff HEAD`                                                                                                              |

If empty and the working tree is clean, fall back to the current branch vs main.

## Output

Write the diff to a temp file and read it with the Read tool — diffs frequently exceed inline output limits.

If reviewing a PR, also read the PR description. If reviewing a commit, also read the commit messages.

Provide a report of the diffs:

Title: <Summary of PR title and commit messages>
Description: <Summary of PR description and commit descriptions>
Size: <size (XS|S|M|L|XL|XXL)>
Diffs: <path to diff file>

Size guidelines:

| Size | Lines   | Files | Review complexity                                                    |
| ---- | ------- | ----- | -------------------------------------------------------------------- |
| XS   | < 50    | 1-2   | **Trivial**: minor typos, documentation, or config tweaks            |
| S    | 50-200  | 3-5   | **Ideal**: single bug fix or small, focused feature                  |
| M    | 200-400 | 6-10  | **Standard**: complete feature or logical refactor                   |
| L    | 400-1k  | 11-20 | **Warning**: high cognitive load, reviewers may start skimming       |
| XL   | 1k-2k   | 20+   | **Critical**: likely needs to be split into smaller, independent PRs |
| XXL  | 2k+     | 50+   | **Dangerous**: defect detection drops significantly                  |

Use whichever dimension gives the larger size (e.g., 3 files but 500 lines = L).
