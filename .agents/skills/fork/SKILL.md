---
name: fork
description: Start or end a forked session for exploring rabbit holes without losing the main conversation's context.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Fork

## `/fork start`

Capture the current context and branch off into a new session.

1. **Snapshot**: write a markdown file to `/tmp/fork-<timestamp>.md` containing:
   - **Goal**: what we're working on
   - **Plan**: current plan or approach (if any)
   - **Progress**: what's been done so far (commits, files changed)
   - **Pending**: what's left to do
   - **Context**: key decisions, constraints, or gotchas discovered along the way
   - **Git state**: current branch, last commit SHA, dirty files
2. **Rename this session**: run `/rename` to give the current session a descriptive name (e.g., "feat: auth middleware") so it's easy to find later.
3. **Instruct the user**: print the file path and a ready-to-paste prompt for the new session:

```
FORKED SESSION — fork file: /tmp/fork-<timestamp>.md

Read the fork file for context on what I was working on. I need to explore: <describe the rabbit hole>.

This is a forked session. Keep track of all findings, commits, and decisions — when done, the user will call `/fork end` to write a summary back to the fork file so the original session can pick it up.
```

Also remind them:

- Run `/rename Fork: <topic>` in the new session to tag it
- Run `/fork end` when done — it writes findings back to the fork file
- Then resume the original session with `claude --continue`

## `/fork end`

Wrap up a rabbit hole session and write findings back to the fork file.

1. **Locate**: find the fork file from the session context (it was read at the start of this session).
2. **Summarize**: append a `## Findings` section to the fork file containing:
   - **Result**: what was discovered or resolved
   - **Commits**: any commits made during the investigation (SHAs + messages)
   - **Impact on original work**: how this affects the plan or pending work from the parent session
3. **Instruct the user**: remind them to resume the original session with `claude --continue` and reference the fork file.
