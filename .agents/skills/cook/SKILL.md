---
name: cook
description: Full autopilot — plan, implement, and polish a task end-to-end.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Cook

End-to-end delivery. Scale the process to the task.

## Pipeline

Triage the task first (see workflow rules), then follow this pipeline. Scale each step to the complexity — a simple task needs a one-line plan, a complex one needs a full design document.

1. **Plan**: run `/plan`. It will ask the user clarifying questions via `AskUserQuestion` before producing a plan — this is mandatory, never skip it even for simple tasks. Scale the plan's depth to the complexity.
2. **Confirm**: present the plan to the user and wait for explicit confirmation before proceeding. If the plan reveals gaps or unfamiliar territory, run `/research` to fill them, then revise the plan and confirm again.
3. **Execute**: run `/execute` with the plan file path. It handles parallelism, subagents, and failure recovery.
4. **Verify**: smoke-test the result — run the tool, command, or test that proves the change works. If it fails, fix and re-verify before proceeding.
5. **Commit**: run `/commit` to stage and commit the changes.
6. **Polish**: run `/polish`. It reviews and amends iteratively until 3x LGTM. If it reveals design gaps that require rethinking (not just code fixes), loop back to step 1. On completion, it produces a report. Skip for simple tasks.
7. **Approve**: present the polish report to the user and STOP. Do NOT proceed to step 8 until the user explicitly says to continue — even in bypass-permissions or auto mode. If they request changes, loop back to step 1 with their feedback.
8. **Ship**: run `/pr`.
