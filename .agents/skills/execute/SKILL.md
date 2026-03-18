---
name: execute
description: Execute a plan file — parse phases, spawn parallel subagents, handle failures.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent, Edit, Write
---

# Execute

Execute a plan file produced by `/plan`. $ARGUMENTS is the path to the plan file.

1. **Parse**: read the plan file. Extract the phases and tasks. Each phase contains independent tasks that can run in parallel.
2. **Execute phase by phase**: for each phase:
   - **Independent tasks** (touching separate files/modules): spawn a subagent per task. Give each subagent the plan file path, its assigned task, and the interfaces it needs to implement.
   - **Interdependent tasks** (shared state, cross-cutting concerns): use an agent team instead — agents share a task list and coordinate directly, which avoids integration conflicts.
   - Wait for all agents in the phase to complete before starting the next phase.
3. **Handle failures**: if a subagent gets stuck or goes off track, kill it, revert only that task's changes (not the entire branch — other tasks in the phase may have succeeded), and spawn a fresh one with adjusted instructions. If a task fails repeatedly (3 attempts), stop and report the failure — don't block other phases that don't depend on it.
4. **Report**: return a summary of completed tasks and any failures.
