---
name: plan
description: Structured design intake for non-trivial tasks. Asks clarifying questions and produces a design proposal.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Plan

Structured intake for complex tasks. The plan lives in a file, not in conversation output. Stay at the architecture level — define boundaries, interfaces, and data flow. Leave implementation details to the implementer.

1. **Understand**: ask clarifying questions using `AskUserQuestion` — this is **mandatory**, never skip it. Use batches of ~4 questions, multiple-choice with 2-4 options + "Other / Not sure" when possible. Iterate until the reasoning framework is filled in — Problem, Goal, Background, Constraints. Do not write code or produce a plan until the user has answered at least one round of questions.
2. **Propose**: write the proposal to a temp file (`/tmp/plan-<timestamp>.md`). Print a markdown link to the file (e.g., `[plan-name.md](/tmp/plan-name.md)`) so the user can click to open it — do not dump the plan into conversation output. Use the reasoning framework from `workflow.md`, structured top-down:
   - **Problem(s)**: what's wrong or missing — the motivation for this work
   - **Goal(s)**: what success looks like (optional if obvious from the problem)
   - **Background**: relevant context to understand the problem/solution — existing architecture, constraints, prior art. Use diagrams (ASCII or Mermaid) for data flow and component relationships when helpful.
   - **Solution**: what we're doing about it and how it integrates with what already exists. Structure as versioned milestones when the work has natural increments (v0 = minimal viable, v1 = full feature, etc.). Each version should be independently shippable. Within a version, group tasks by dependency — independent tasks are parallelizable. Define interfaces (API contracts, data shapes, protocols) at boundaries where parallel work can happen. Use diagrams (ASCII or Mermaid) for architecture and data flow when helpful.
   - **Alternative(s) considered**: other approaches evaluated and why not (optional, include when non-obvious)
   - **Implementation details**: specific file paths, configs, schemas, code snippets that clarify the solution. Optional — the plan should be understandable without it.
3. **Iterate**: when the user requests changes, update the file in place. Never reprint the full plan — just summarize what changed.
4. **Confirm**: wait for user confirmation before implementing.
