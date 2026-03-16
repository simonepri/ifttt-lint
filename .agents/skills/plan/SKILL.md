---
name: plan
description: Structured design intake for non-trivial tasks. Asks clarifying questions and produces a design proposal.
---

# Plan

Structured intake for complex tasks. The plan lives in a file, not in conversation output. Stay at the architecture level — define boundaries, interfaces, and data flow. Leave implementation details to the implementer.

1. **Understand**: ask clarifying questions iteratively (batches of ~4, multiple-choice when possible) to fill in the reasoning framework — Problem, Goal, Background, Constraints. Do not write code until the user says "Proceed".
2. **Propose**: write the proposal to a temp file (`/tmp/plan-<timestamp>.md`). Open the file in the IDE or print the path — do not dump the plan into conversation output. Structure top-down:
   - **Summary**: one paragraph, what we're building and why
   - **Architecture**: components, their responsibilities, and how they communicate. Use diagrams (ASCII or Mermaid) for data flow and component relationships.
   - **Interfaces**: API contracts between components — function signatures, data shapes, protocols. This is the boundary where parallel work can happen independently.
   - **Phases**: group work into phases based on dependencies. Within a phase, tasks are independent and parallelizable. Between phases, there's a dependency. Each task: one sentence of what it does and which component/interface it implements.
   - **Alternatives considered**: other approaches and why not (if non-obvious)
3. **Iterate**: when the user requests changes, update the file in place. Never reprint the full plan — just summarize what changed.
4. **Confirm**: wait for user confirmation before implementing.
