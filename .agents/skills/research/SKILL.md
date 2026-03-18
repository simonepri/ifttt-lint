---
name: research
description: Investigate a topic, library, or approach before planning. Combines user context, prior knowledge, and external sources.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Research

Structured exploration to ground decisions in evidence. Use before `/plan` or whenever facing unfamiliar territory. $ARGUMENTS is the topic or question to investigate.

1. **Scope**: what do we need to learn? Break the topic into specific questions. If the user gave a broad topic, narrow it down — "how does X work?" is better than "tell me about X".
2. **Prior knowledge**: start with what you already know. State it explicitly so the user can correct misconceptions early.
3. **External sources**: for each open question, gather evidence following the Research rule in `workflow.md` — Context7 for libraries/APIs, web search for patterns and recommendations, source code for internals.
4. **Synthesize**: write findings to a temp file (`/tmp/research-<timestamp>.md`). Structure:
   - **Topic**: what we investigated
   - **Key findings**: numbered list of concrete answers to the scoped questions
   - **Recommendations**: what approach to take and why, based on the evidence
   - **Open questions**: anything still unclear that needs user input or deeper investigation
5. **Present**: open the file in the IDE or print the path. Summarize the key findings and recommendations in conversation — keep it brief, the file has the details.
