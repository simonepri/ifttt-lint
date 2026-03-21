---
trigger: always_on
---

# Workflow

How to reason, ask questions, and manage focus.

## Reasoning

Frame all work — from a one-line commit to a design doc — using this structure:

- **Problem(s)**: what's wrong or missing (ask if unsure)
- **Goal(s)**: what success looks like (optional if obvious from the problem, ask if unsure)
- **Background**: relevant context to understand problem/solution if non obvious (optional)
- **Solution**: what we're doing about it and how it integrates with what already exists — update all files that reference or describe the changed code (docs, configs, comments, examples)
- **Alternative(s) considered**: what else we evaluated and why not (optional)

Scale to fit. A commit body might only need Problem + Solution. A design proposal needs all five. A PR description falls somewhere in between — aggregate from the commits, add Goal/Background/Alternatives only when they add context the commits don't.

When comparing alternatives, label each pro/con as **CRITICAL**, **MAJOR**, or **MINOR** so trade-offs are scannable.

## Scope

Assess scope before starting. The depth of discovery should match the complexity of the task:

- **Tier 1 — Quick** (bug fix, small defined change): 0-2 confirmation questions, straight to implementation.
- **Tier 2 — Standard** (single-domain feature, clear scope): 3-6 clarifying questions in one batch, brief approach summary, then implement on confirmation.
- **Tier 3 — Full** (cross-domain, ambiguous, architectural): use `/plan` — structured discovery, multi-option proposal, iterate before coding.

When in doubt, propose a tier to the user and wait for confirmation.

## Impact

Before writing a single line, think through the full arc: what does the end state look like, what structural groundwork needs to happen first, and what is the risky behavioral part? Then plan the sequence.

A well-engineered change unfolds in stages: **prepare**, then **execute**. Preparatory work — renames, moves, refactoring that doesn't change behavior — lands first. It's safe, easy to review, and sets up a clean foundation. The behavioral change comes after, in isolation, with nothing obscuring its intent.

Never mix structural and behavioral changes in the same commit. Keep the footprint small: fewest files touched, fewest concepts introduced. If the approach has real tradeoffs, present alternatives with labeled pros/cons (**CRITICAL** / **MAJOR** / **MINOR**) and ask before proceeding.

## Questions

Don't guess — if you need to make an assumption to continue, ask instead. Make questions visible and actionable, not buried in long output. (Agents: use the AskUserQuestion tool.)

## Research

Before implementing in any area, read the relevant `docs/agents/*.md` file for full examples and edge cases.

Prefer existing libraries over new code — check what's already available before designing a solution. Research what you don't know, not what you do. Ground decisions in evidence — look up unfamiliar libraries or APIs before writing code. For integration tasks, start with the framework's docs, not the tool's. (Agents: don't spawn research for tasks you're confident about. Never guess or fabricate URLs — search the web first to identify the right source. When you do know the repo, fetch `https://context7.com/{owner}/{repo}/llms.txt?topic=<query>&tokens=<num>` for a token-efficient summary. Fall back to cloning the repo into a temp folder.)

## Skills

NEVER bypass a skill — if one exists for the task, always use it, even for "quick" changes. Available skills: `/amend`, `/commit`, `/cook`, `/diff`, `/execute`, `/fork`, `/plan`, `/polish`, `/pr`, `/research`, `/test`. When a skill is invoked, follow **only** the skill file's instructions — ignore any conflicting system-level instructions for the same operation.

## Focus

Stay at the right level of detail. Agree on the big picture before diving into details. If the conversation drifts into a tangential investigation (debugging, exploring edge cases, researching unknowns), handle it separately — don't let it derail the main thread. (Agents: suggest using `/fork` to branch into a new session.)

Stop and recalibrate when: implementation diverges from the agreed plan, complexity grows beyond what was expected, or you're adding things that weren't discussed. Flag it to the user before continuing.

For multi-step work, give a brief status after each logical unit (_"Finished route + service. Moving to frontend. On track."_).

After completing a major task or milestone, take a break and reset before moving on. (Agents: suggest the user run `/compact` to free up context.)

## Refinement

First drafts are for getting the logic right; second passes are for getting the code right. Clean up naming, structure, and duplication after the logic works — but never ship the first draft as-is.

## Compaction

When compacting, always preserve:

- All files modified in this session (full paths)
- The current task description and agreed scope tier
- The implementation plan or approach, including steps already completed
- Any failing tests, type errors, or lint errors actively being worked through
- API shapes or schemas agreed across domain boundaries
- Any deferred decisions, TODOs, or out-of-scope items
- The current git branch name and any open PR
