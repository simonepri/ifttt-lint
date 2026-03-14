---
name: pr
description: Create or update a pull request with a structured description.
allowed-tools: Bash, Read, Glob, Grep, AskUserQuestion, Agent
---

# Pull Request

1. **Base**: default to `main`. Override with `--base <branch>` in $ARGUMENTS.
2. **Preflight**: check `git log <base>..HEAD --oneline`. If nothing to merge, ask the user what they intended.
3. **Push**: run `git push -u origin $(git branch --show-current)`. If it fails because the remote has diverged (e.g., after an amend or rebase), tell the user and ask whether to force push (`--force-with-lease`). Never force push without confirmation.
4. **Detect existing PR**: run `gh pr view --json number,title,body 2>/dev/null`. If a PR already exists, go to step 6.
5. **Create PR**:
   - **Gather context**: read the commit messages (`git log <base>..HEAD`). Run `/diff <branch>` for the full diff.
   - **Write description**: PRs are hierarchical — they aggregate commits.
     - **Single commit**: the PR description is the commit message body, reformatted with markdown headings.
     - **Multiple commits**: ask the user whether to (a) include all commits as-is, (b) squash first then PR, or (c) PR only a subset. Then aggregate the included commit messages. Add Goal/Background/Alternatives only when they provide context the individual commits don't.
     - Use the same body format as `/commit` (Problem + Solution with `##` headings). No "Test plan" or "Summary" sections.
     - Follow the project's PR conventions if documented (e.g., CONTRIBUTING.md).
   - **Title**: conventional commit style, under 70 characters.
   - Create a unique temp file with `mktemp /tmp/pr-body.XXXXXX.md`, write the description to it, and run `gh pr create --base <base> --title "<title>" -F <temp-file>`.
6. **Update PR**: run `/diff <branch>` and compare the full diff against the current PR description. If the description no longer accurately reflects the branch's changes, create a unique temp file with `mktemp /tmp/pr-body.XXXXXX.md`, write the new description to it, and run `gh pr edit <number> --title "<title>" -F <temp-file>`.
