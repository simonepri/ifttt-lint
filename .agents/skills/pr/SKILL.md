---
name: pr
description: Create a pull request with a structured description.
---

# Create Pull Request

1. **Base**: default to `main`. Override with `--base <branch>` in $ARGUMENTS.
2. **Preflight**: check `git log <base>..HEAD --oneline`. If nothing to merge, ask the user what they intended.
3. **Gather context**: read the commit messages (`git log <base>..HEAD`). Run `/diff <branch>` for the full diff.
4. **Write description**: PRs are hierarchical — they aggregate commits.
   - **Single commit**: the PR description is the commit message body. Don't rewrite it.
   - **Multiple commits**: ask the user whether to (a) include all commits as-is, (b) squash first then PR, or (c) PR only a subset. Then aggregate the included commit messages. Add Goal/Background/Alternatives only when they provide context the individual commits don't.
   - Follow the project's PR conventions if documented (e.g., CONTRIBUTING.md).
5. **Push and create**:

```bash
git push -u origin $(git branch --show-current)
gh pr create --base <base> --title "<title>" --body "$(cat <<'EOF'
<description>
EOF
)"
```

**Title**: conventional commit style, under 70 characters.
