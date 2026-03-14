# let-em-cook

> 🤌🏻 Teach AI how to cook (code) the way you like it.

## Rules

Read and follow the rules in `.agents/rules/` before writing any code:

- `.agents/rules/code.md` — Code style principles (clarity > simplicity > concision > maintainability > consistency)
- `.agents/rules/workflow.md` — How to reason, ask questions, and manage focus

## Skills

Invoke these workflows when the user calls them. Read the skill file and follow **only** its instructions (see "Skills" rule in `.agents/rules/workflow.md`):

| Command     | Skill file                         |
| ----------- | ---------------------------------- |
| `/cook`     | `.agents/skills/cook/SKILL.md`     |
| `/research` | `.agents/skills/research/SKILL.md` |
| `/plan`     | `.agents/skills/plan/SKILL.md`     |
| `/execute`  | `.agents/skills/execute/SKILL.md`  |
| `/commit`   | `.agents/skills/commit/SKILL.md`   |
| `/amend`    | `.agents/skills/amend/SKILL.md`    |
| `/review`   | `.agents/skills/review/SKILL.md`   |
| `/polish`   | `.agents/skills/polish/SKILL.md`   |
| `/pr`       | `.agents/skills/pr/SKILL.md`       |
| `/diff`     | `.agents/skills/diff/SKILL.md`     |
| `/fork`     | `.agents/skills/fork/SKILL.md`     |
