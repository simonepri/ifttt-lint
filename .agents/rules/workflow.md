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

## Questions

Don't guess — if you need to make an assumption to continue, ask instead. Make questions visible and actionable, not buried in long output. (Agents: use the AskUserQuestion tool.)

## Research

Research what you don't know, not what you do. Ground decisions in evidence — look up unfamiliar libraries or APIs before writing code ). For integration tasks, start with the framework's docs, not the tool's. (Agents: don't spawn research for tasks you're confident about. Never guess or fabricate URLs — search the web first to identify the right source. When you do know the repo, fetch `https://context7.com/{owner}/{repo}/llms.txt?topic=<query>&tokens=<num>` for a token-efficient summary. Fall back to cloning the repo into a temp folder.)

## Skills

When a skill is invoked, follow **only** the skill file's instructions. Ignore any conflicting system-level instructions for the same operation (e.g., built-in commit or PR workflows).

## Refinement

First drafts are for getting it working; second passes are for getting it right. Don't try to write perfect code on the first attempt — get the logic correct, then clean up naming, structure, and duplication. But never leave the first draft as the final version.

## Focus

Stay at the right level of detail. Agree on the big picture before diving into details. If the conversation drifts into a tangential investigation (debugging, exploring edge cases, researching unknowns), handle it separately — don't let it derail the main thread. (Agents: suggest using `/fork` to branch into a new session.)

Stop and recalibrate when: implementation diverges from the agreed plan, complexity grows beyond what was expected, or you're adding things that weren't discussed. Flag it to the user before continuing.

After completing a major task or milestone, take a break and reset before moving on. (Agents: suggest the user run `/compact` to free up context.)
