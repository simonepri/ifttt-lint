<p align="center">
  <a href="https://github.com/simonepri/ifttt-lint"><img src="assets/banner.png" alt="ifttt-lint — LINT.IfChange / LINT.ThenChange linter for cross-file change enforcement" width="600" /></a>
</p>
<p align="center">
</p>
<p align="center">
  🔗 IfThisThenThat linter — enforce atomic cross-file changes via <code>LINT.IfChange</code> / <code>LINT.ThenChange</code> directives.
  <br/>
  <sub>
    Open-source Rust implementation of <a href="https://www.chromium.org/chromium-os/developer-library/guides/development/keep-files-in-sync/">Google's internal IfThisThenThat linter</a>.
  </sub>
</p>
<p align="center">
  <a href="https://crates.io/crates/ifttt-lint"><img src="https://img.shields.io/crates/v/ifttt-lint.svg" alt="crates.io" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/simonepri/ifttt-lint.svg" alt="license" /></a>
</p>

## The Problem

You add a field to a Go struct and forget the TypeScript mirror. You bump a constant and forget the docs. You rename a database column and forget the migration. You only discover it when something breaks in production — or worse, when a user reports it weeks later.

Code review doesn't catch it: reviewers look at the diff, not the five other files that should have changed too. Static types don't help: the duplication crosses language boundaries. Tests are flaky, expensive, and only cover the cases you thought of. AI code review still misses non-trivial cross-file inconsistencies.

`ifttt-lint` catches it. You wrap co-dependent sections in `LINT.IfChange` / `LINT.ThenChange` comment directives — the IfChangeThenChange pattern. When a diff touches one side but not the other, the tool fails — before the change reaches production. It's stupidly simple, and that's why it works.

The pattern is not new — [Google's internal IfThisThenThat linter](https://www.chromium.org/chromium-os/developer-library/guides/development/keep-files-in-sync/) has enforced it across Chromium, TensorFlow, Fuchsia, virtually any internal Google project for over a decade. Outside Google, the same instinct shows up as informal comments — [ _"if you change this, also update…"_](https://github.com/search?q=%28%22%2F%2F+if+you+change%22+OR+%22%23+if+you+change%22+OR+%22%2F%2F+if+you+modify%22+OR+%22%23+if+you+modify%22+OR+%22%2F%2F+when+changing%22+OR+%22%23+when+changing%22+OR+%22%2F%2F+when+you+change%22+OR+%22%23+when+you+change%22+OR+%22%2F%2F+if+this+changes%22+OR+%22%23+if+this+changes%22%29+AND+%28%22also+change%22+OR+%22also+update%22+OR+%22must+be+updated%22+OR+%22must+also+be%22+OR+%22needs+to+be+updated%22+OR+%22in+sync%22%29&type=code) with no enforcement.

_But shouldn't DRY eliminate this?_ In theory, yes — you could generate docs from code, derive migrations from schemas, template config files. In practice, that level of automation is often impractical: the duplication is too small to justify a pipeline, crosses too many boundaries, or lives in systems you don't control. `ifttt-lint` is the safety net for those gaps.

_What does that look like in practice?_ This repo dogfoods its own directives to keep the tool version in sync across [`Cargo.toml`](Cargo.toml), the [pre-commit config](#pre-commit-recommended), and the [CI release pipeline](.github/workflows/ci-cd.yml). If any of the three drifts, the linter catches it before merge.

## Usage

Add directives as comments in any language — the tool auto-detects comment styles based on file extension.

### Keep code and docs in sync

Your upload limit is defined in code and referenced in the API docs. Label both sides and link them — if one changes, the other must too:

<table>
<tr>
<th><code>config/upload.py</code></th>
<th><code>docs/api.md</code></th>
</tr>
<tr>
<td>

```python
# LINT.IfChange(upload_limit)
MAX_UPLOAD_SIZE_MB = 50
# LINT.ThenChange(//docs/api.md:upload_limit)
```

</td>
<td>

```markdown
<!-- LINT.IfChange(upload_limit) -->

Files up to 50 MB are accepted.

<!-- LINT.ThenChange(//config/upload.py:upload_limit) -->
```

</td>
</tr>
</table>

Bump the limit to 100 MB but forget the docs? The linter catches it:

```
config/upload.py:1: warning: changes in this block may need to be reflected in docs/api.md:upload_limit
```

### Sync across language boundaries

When types cross language boundaries, a shared schema language (Protocol Buffers, Thrift, GraphQL) is the best solution. But not every project uses one — and even when it does, hand-written types often exist alongside generated ones. For those cases, link the two sides directly:

<table>
<tr>
<th><code>api/types.go</code></th>
<th><code>web/src/types.ts</code></th>
</tr>
<tr>
<td>

```go
// LINT.IfChange(user_response)
type UserResponse struct {
    ID    string `json:"id"`
    Name  string `json:"name"`
    Email string `json:"email"`
}
// LINT.ThenChange(//web/src/types.ts:user_response)
```

</td>
<td>

```typescript
// LINT.IfChange(user_response)
interface UserResponse {
  id: string;
  name: string;
  email: string;
}
// LINT.ThenChange(//api/types.go:user_response)
```

</td>
</tr>
</table>

### Link multiple targets

A rate limit touches the database, the docs, and an alerting threshold in the same file. List all dependents — the tool checks every target:

```python
# LINT.IfChange
RATE_LIMIT_RPS = 100
# LINT.ThenChange(
#     //db/migrations/002_rate_limits.sql,
#     //docs/api.md:rate_limits,
#     :alert_threshold,
# )
```

### Sync within a file

Serialize and deserialize must stay in lockstep — use `:label` to reference another section in the same file:

```python
# LINT.IfChange(serialize_event)
def serialize_event(event: Event) -> bytes: ...
# LINT.ThenChange(:deserialize_event)

# LINT.IfChange(deserialize_event)
def deserialize_event(data: bytes) -> Event: ...
# LINT.ThenChange(:serialize_event)
```

### Works in any comment style

Directives use whatever comment syntax the file extension implies — SQL, YAML, HTML, and [40+ languages](#supported-languages):

```sql
-- LINT.IfChange(schema)
CREATE TABLE users (id UUID, name TEXT, email TEXT);
-- LINT.ThenChange(//api/types.go:user_response)
```

```yaml
# LINT.IfChange(deploy_config)
replicas: 3
# LINT.ThenChange(//docs/runbook.md:scaling)
```

### Safe in documentation

Directives inside fenced code blocks (` ``` `) are ignored — the linter won't fire on examples in markdown files or doc comments. This README itself contains dozens of `LINT.IfChange` examples and passes `ifttt-lint` cleanly.

A documentation file like this is safe:

````markdown
## Keeping files in sync

Use `LINT.IfChange` / `LINT.ThenChange` to link co-dependent sections:

```python
# LINT.IfChange(upload_limit)
MAX_UPLOAD_SIZE_MB = 50
# LINT.ThenChange(//docs/api.md:upload_limit)
```
````

The directives above are inside a fenced code block, so the linter skips them entirely. The same applies to code blocks inside doc comments (e.g., Rust `///`, Python docstrings with embedded examples).

## Performance

`ifttt-lint` is designed to add negligible overhead to your CI pipeline. In **diff mode**, only files touched by the diff are read and parsed. The optional **structural validity pass** (triggered by `[FILES]...`) validates that all targets and labels referenced in the listed files exist on disk, reading only the files actually passed — no full-repo scan required.

When the diff **deletes** (or renames) a file, a **reverse-lookup pass** walks the repo to find surviving files that still reference the deleted target. This is the only scenario that triggers a repo-wide scan. The walk runs once (not once per deleted file) and uses two cheap substring filters to skip the vast majority of files before parsing: first, does the file contain `LINT.`? If not, skip. Then, does it mention any of the deleted paths? If not, skip. Files already parsed in earlier passes reuse their cached result. The walk uses `git grep` to discover candidates, which only searches tracked files and respects `.gitignore`. Untracked files containing LINT directives that reference a deleted target will not be caught — commit or stage them first.

**Real-world** (structural validation, M-series MacBook):

| Repository                                             | Tracked files | Files with directives | 1 thread | 2 threads        | 4 threads    |
| ------------------------------------------------------ | ------------: | --------------------: | -------- | ---------------- | ------------ |
| [Chromium](https://github.com/chromium/chromium)       | 488k (~3.9GB) |          1.7k (~39MB) | 1.9 s    | **0.9 s** (2.0×) | 1.2 s (1.6×) |
| [TensorFlow](https://github.com/tensorflow/tensorflow) |  36k (~402MB) |          244 (~5.3MB) | 0.3 s    | **0.2 s** (1.3×) | 0.3 s (1.3×) |

<sub>Structural validation on M-series MacBook. Reproduce with `cargo smoke`.</sub>

The default thread count is **2** (`--threads 0`), which gives near-optimal throughput. Higher counts hit filesystem I/O contention and degrade.

## Setup

### pre-commit (recommended)

<!-- LINT.IfChange(version-pre-commit) -->

```yaml
- repo: https://github.com/simonepri/ifttt-lint
  rev: v0.6.0
  hooks:
    - id: ifttt-lint
    - id: ifttt-lint-diff
```

<!-- LINT.ThenChange(//Cargo.toml:version, //.github/workflows/ci-cd.yml:version) -->

Two hooks serve different purposes:

- **`ifttt-lint`** — runs at every commit on the staged files. Checks that all `ThenChange` targets and labels exist on disk, directives are properly paired, and syntax is valid. Also supports `pre-commit run --all-files` for full-repo structural scans.

- **`ifttt-lint-diff`** — runs at every push on all files in the diff range. Checks that co-dependent files are updated together. Supports `NO_IFTTT` suppression via commit messages. Mirrors the `pull_request` GitHub Actions check — same diff range, same suppression mechanism.

### GitHub Actions

<!-- LINT.IfChange(version-github-action) -->

```yaml
on:
  push:
    branches: [main]
  pull_request:

jobs:
  ifttt-lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: simonepri/ifttt-lint@v0.6.0
```

<!-- LINT.ThenChange(//Cargo.toml:version, //.github/workflows/ci-cd.yml:version) -->

The action mirrors the two hooks:

- **`pull_request`** — diff validation equivalent to `ifttt-lint-diff`. Validates co-changes across all commits in the PR. Supports `NO_IFTTT` suppression via commit messages.
- **`push`** — structural validation on all tracked files, equivalent to `ifttt-lint '*'`. Use `on.push.branches` to control which branches run it.

## CLI Reference

```
ifttt-lint [OPTIONS] [FILES]...
```

| Argument   | Description                                                                                                                                                                                                                                                |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FILES...` | Files to validate structurally: checks that every `ThenChange` target and label exists on disk, regardless of whether the file was modified. Supports glob patterns (e.g. `'*'`) — resolved internally via `git ls-files` to avoid shell `ARG_MAX` limits. |

| Option                   | Description                                                                                                                                      |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `-d, --diff <RANGE>`     | Git ref range to diff (e.g. `main...HEAD`)                                                                                                       |
| `-t, --threads <N>`      | Worker thread count (default: 2; 0 = same as 2)                                                                                                  |
| `-i, --ignore <PATTERN>` | Permanently ignore target pattern, repeatable (glob syntax)                                                                                      |
| `--strict=false`         | Accept bare and single-`/` paths in `ThenChange` targets (in addition to `//`). Required for codebases that use Google-internal path conventions |
| `-f, --format <FMT>`     | Output format: `pretty` (default), `json`, `plain`                                                                                               |

| Exit Code | Meaning                             |
| --------- | ----------------------------------- |
| `0`       | No errors                           |
| `1`       | Lint errors found                   |
| `2`       | Fatal error (bad diff, I/O failure) |

## Output

The default `pretty` format uses standard `file:line: severity: message` syntax, compatible with most editors and CI systems.

### Diff-based validation

You bump `MAX_UPLOAD_SIZE_MB` to 100 but forget the docs:

```
config/upload.py:1: warning: changes in this block may need to be reflected in docs/api.md:upload_limit
```

You add an `Avatar` field to the Go struct but forget the TypeScript mirror:

```
api/types.go:2: warning: changes in this block may need to be reflected in web/src/types.ts:user_response
```

### Structural validation

When passing `[FILES]...`, the tool validates that directive targets and labels exist on disk — even if the file wasn't part of the diff. A `ThenChange` pointing to a deleted file:

```
api/types.go:7: warning: target file not found: web/src/old_types.ts
```

A `ThenChange` referencing a label that was renamed:

```
config/upload.py:3: warning: label upload_limit not found in docs/api.md
```

Malformed directives are caught as parse errors:

```
config/upload.py:5: error: LINT.ThenChange without preceding IfChange
```

## Suppressing Findings

When you intentionally skip diff-based `ThenChange` checks for a commit, add `NO_IFTTT=<reason>` to the commit message:

```
feat: raise upload limit to 100 MB

NO_IFTTT=docs will be updated in a follow-up
```

> [!NOTE]
> In `--diff` mode (pre-push hook, CI), the tool parses `NO_IFTTT` tags from commit messages and suppresses diff-based findings for that range. Structural validity checks (from `[FILES]...`) and reverse-lookup for deleted targets still run.

To permanently ignore targets, use `--ignore`:

```bash
ifttt-lint --ignore "generated/*" --ignore "*.lock"
```

## Directive Reference

`ifttt-lint` implements [Google's `LINT.IfChange` / `LINT.ThenChange` directive syntax](https://www.chromium.org/chromium-os/developer-library/guides/development/keep-files-in-sync/)<sup>†</sup>.

| Directive                       | Description                                                     |
| ------------------------------- | --------------------------------------------------------------- |
| `LINT.IfChange`                 | Marks the start of a watched region                             |
| `LINT.IfChange(label)`          | Watched region with a named label (targetable from other files) |
| `LINT.ThenChange(//path)`       | End of watched region; requires target file to be modified      |
| `LINT.ThenChange(//path:label)` | Requires changes within a specific label range in the target    |
| `LINT.ThenChange(:label)`       | Same-file label reference                                       |
| `LINT.ThenChange(//a, //b)`     | Multiple targets (comma-separated)                              |

<sub>† `ifttt-lint` enforces [stricter path rules](#path-rules) than Google's internal linter by default — use `--strict=false` for Google-compatible behavior.</sub>

### Path Rules

- All file paths must start with `//` (project-root-relative)
- `:` separates file path from label (splits on last `:`)
- `:label` alone means same-file reference

Use `--strict=false` for Google-compatible behavior — bare paths (`path/to/file`), single-`/` paths (`/path/to/file`), and explicit same-file path references (`//same-file.h:label` instead of `:label`) are all accepted without warnings.

### Label Format

Labels must start with a letter, followed by letters, digits, underscores, dashes, or dots. For example: `upload_limit`, `user-response`, `section2`, `Payments.Pix.Result`.

## Validation Rules

`ifttt-lint` runs up to three validation passes depending on how it's invoked. This section documents when each pass runs and what it checks.

### CLI modes

| Invocation                         | Hook stage   | What runs                                                                                                                        |
| ---------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| `ifttt-lint` (no args)             | —            | Nothing — exits 0 with a hint                                                                                                    |
| `ifttt-lint FILES…` (no `--diff`)  | `pre-commit` | Structural validation on listed files only                                                                                       |
| `ifttt-lint '*'` (no `--diff`)     | CI, manual   | Structural validation on all tracked files (glob expanded via `git ls-files`)                                                    |
| `ifttt-lint --diff REF FILES…`     | —            | Structural validation on listed files. Diff validation scoped to listed files. Reverse lookup for deleted files and stale labels |
| `ifttt-lint --diff REF` (no files) | `pre-push`   | Structural + diff validation on all files in the diff. Reverse lookup for deleted files and stale labels                         |

### Diff-based validation

When an `IfChange`…`ThenChange` block is present in a changed file, the tool checks whether the **guarded content** (lines between the directives) was modified. If it was, every `ThenChange` target must also show changes in the same diff — otherwise a finding is reported.

Changes to the directive lines themselves (adding a new pair, renaming a label, adding or removing a `ThenChange` target) do **not** trigger validation — only content between the directives matters.

**Fires:**

A field is added to the Go struct but the TypeScript mirror is not updated:

```diff
  // LINT.IfChange(user_response)
  type UserResponse struct {
      ID    string `json:"id"`
      Name  string `json:"name"`
+     Avatar string `json:"avatar"`
  }
  // LINT.ThenChange(//web/src/types.ts:user_response)
```

```
api/types.go:1: warning: changes in this block may need to be reflected in web/src/types.ts:user_response
```

Content is modified and a new target is added in the same diff — all targets (including the new one) must reflect the change, because you're declaring a dependency while simultaneously changing the content it guards:

```diff
  // LINT.IfChange(upload_limit)
- MAX_UPLOAD_SIZE_MB = 50
+ MAX_UPLOAD_SIZE_MB = 100
- // LINT.ThenChange(//docs/api.md:upload_limit)
+ // LINT.ThenChange(
+ //     //docs/api.md:upload_limit,
+ //     //alerts/thresholds.yaml:upload_limit,
+ // )
```

```
config/upload.py:1: warning: changes in this block may need to be reflected in docs/api.md:upload_limit
config/upload.py:1: warning: changes in this block may need to be reflected in alerts/thresholds.yaml:upload_limit
```

**Does not fire:**

Adding a new directive pair around existing code — the directive is being established, not the content changed:

```diff
+ // LINT.IfChange(speed_threshold)
  SPEED_THRESHOLD_MPH = 88
+ // LINT.ThenChange(//docs/delorean.md:speed_threshold)
```

Adding a new target to an existing directive — directive metadata changed, not guarded content:

```diff
  // LINT.IfChange(rate_limit)
  RATE_LIMIT_RPS = 100
- // LINT.ThenChange(//docs/api.md:rate_limits)
+ // LINT.ThenChange(
+ //     //docs/api.md:rate_limits,
+ //     //alerts/thresholds.yaml:rate_limits,
+ // )
```

Renaming a label — directive metadata changed; stale references are caught by the reverse lookup instead:

```diff
- // LINT.IfChange(old_name)
+ // LINT.IfChange(new_name)
  SPEED_THRESHOLD_MPH = 88
  // LINT.ThenChange(//docs/delorean.md:speed_threshold)
```

Both sides updated in the same diff — the target already reflects the change:

```diff
  // LINT.IfChange(upload_limit)
- MAX_UPLOAD_SIZE_MB = 50
+ MAX_UPLOAD_SIZE_MB = 100
  // LINT.ThenChange(//docs/api.md:upload_limit)
```

```diff
  <!-- LINT.IfChange(upload_limit) -->
- Files up to 50 MB are accepted.
+ Files up to 100 MB are accepted.
  <!-- LINT.ThenChange(//config/upload.py:upload_limit) -->
```

Suppressed via `NO_IFTTT` in the commit message — explicitly opted out:

```
feat: raise upload limit to 100 MB

NO_IFTTT=docs will be updated in a follow-up
```

### Suppression

`NO_IFTTT=<reason>` in any commit message in the scanned range suppresses diff-based validation for the entire range. See [Suppressing Findings](#suppressing-findings) for syntax details.

**Scope** — each context scans exactly one range:

| Context                            | Diff range                                    | Commit messages scanned                |
| ---------------------------------- | --------------------------------------------- | -------------------------------------- |
| pre-push hook                      | `FROM_REF..TO_REF` (all unpushed commits)     | All unpushed commits                   |
| Pull request (CI)                  | `BASE_SHA...HEAD_SHA` (merge-base to PR head) | All commits in the PR                  |
| Push to main (squash merge, CI)    | `BEFORE..HEAD` (1 commit)                     | That squashed commit                   |
| Push to main (rebase merge, N, CI) | `BEFORE..HEAD` (all N commits)                | All N commits                          |
| Push to main (merge commit, CI)    | `BEFORE..HEAD` (merge + PR branch commits)    | Merge commit and all PR branch commits |

Structural validation and deleted-file reverse lookup always run regardless of `NO_IFTTT`. The tag has no effect without `--diff`.

### Structural validation

When files are passed as positional arguments (`FILES…`), the tool checks directive structure regardless of the diff. This catches issues that diff-based validation can't see — broken references, missing targets, malformed syntax.

| Check                                  | Example message                               |
| -------------------------------------- | --------------------------------------------- |
| ThenChange target file doesn't exist   | `target file not found: web/src/old_types.ts` |
| ThenChange label not found in target   | `label upload_limit not found in docs/api.md` |
| IfChange without matching ThenChange   | `LINT.IfChange without matching ThenChange`   |
| ThenChange without preceding IfChange  | `LINT.ThenChange without preceding IfChange`  |
| Duplicate IfChange labels in same file | `duplicate LINT.IfChange label foo`           |

### Reverse lookup

When a file is **deleted**, the tool walks the repository to find surviving files that still reference it via `ThenChange` and flags each as a stale reference.

When a file is **modified** and may have had `IfChange` labels added, renamed, or removed, the tool verifies that all `ThenChange` references from other files still point to valid labels. This catches label renames and deletions — including labels moved to a different file.

```
api/types.go:7: warning: target file not found: web/src/old_types.ts
config/upload.py:3: warning: label old_name not found in constants.py
```

Reverse lookup always runs globally — it is not scoped by the file list.

## Supported Languages

Comment style is detected by file extension. The full language registry with skip-pattern documentation lives in [`src/languages.rs`](src/languages.rs) — 43 entries covering 100+ file extensions.

| Style      | Languages                                                                                                                        |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `//` `/*`  | C/C++, C#, Dart, Go, Groovy, Java, JavaScript, Kotlin, Objective-C, Protobuf, Rust, Scala, SCSS, Swift, TypeScript               |
| `#`        | CMake, Dockerfile, Elixir, GN, GraphQL, Makefile, Nix, Perl, PowerShell, Python, R, Ruby, Shell, Starlark, Terraform, TOML, YAML |
| `<!-- -->` | HTML, Markdown, XML                                                                                                              |
| `--`       | Haskell, Lua, SQL                                                                                                                |
| `;`        | Lisp / Clojure                                                                                                                   |
| `%`        | LaTeX                                                                                                                            |
| `/* */`    | CSS                                                                                                                              |

Multi-syntax: Vue/Svelte (`//`, `/*`, `<!--`), PHP (`//`, `/*`, `#`), Terraform (`#`, `//`, `/*`).

Unknown extensions fall back to `//`, `/*`, `#`.

## FAQ

### Why not use types, codegen, or a shared schema?

When the duplication lives within a single language, you should absolutely use types, shared constants, or code generation. For cross-language type contracts, a shared schema language (Protocol Buffers, Thrift, GraphQL) is the gold standard. `IfChange`/`ThenChange` is for the gaps that remain — code-to-docs (constant ↔ prose), code-to-config (source ↔ build file), hand-written types alongside generated ones, or encode/decode pairs within the same file. A lightweight comment directive beats an over-engineered abstraction when the duplication is small, infrequent, or crosses boundaries that no schema language covers.

### Can I use this in a monorepo with multiple languages?

Yes — that's the primary use case. Directives work across any file types in the [supported languages](#supported-languages) table. Paths are project-root-relative (`//`), so they work regardless of where the files live in the tree.

### Does it work across repositories?

No — paths are project-root-relative (`//`), so all linked files must live in the same repository. Cross-repo dependencies are a fundamentally harder problem (versioning, release cadence, ownership boundaries) that a comment directive can't solve. If you need cross-repo coordination, consider shared packages with versioned contracts, or a schema registry. If you have ideas on how cross-repo support could work, [open an issue](../../issues).

### What happens when I delete a file that's referenced by a `ThenChange`?

The tool runs a reverse-lookup pass: it walks the repo to find surviving files that still reference the deleted path and flags them as errors. This ensures stale references don't silently accumulate.

### How do I handle merge conflicts in LINT directives?

Resolve them like any other merge conflict. The tool validates the final state of the file, not the merge process. If the resolved file has valid directives pointing to valid targets, it passes.

### Does it work with Mercurial, Perforce, or other VCS?

Currently only Git is supported. The core validation logic is VCS-agnostic — all VCS operations (diffs, file reads, file search) go through a [`VcsProvider` trait](src/vcs.rs), and Git is the only implemented backend ([`src/vcs_git.rs`](src/vcs_git.rs)). Adding Mercurial, Perforce, or another VCS means implementing that trait — no changes to the validation engine are needed. PRs welcome; [open an issue](../../issues) to discuss.

### My language isn't in the supported list — can I add it?

Yes, please contribute! Adding a new language is just a new entry in the [comment-style table](src/languages.rs) — no changes to the parser or validation engine. PRs welcome; [open an issue](../../issues) if you're unsure about the comment syntax.

### Are there other implementations?

[if-changed](https://github.com/mathematic-inc/if-changed), [ifttt-lint](https://github.com/ebrevdo/ifttt-lint), and [ifchange](https://github.com/slnc/ifchange) exist but use different syntax and aren't validated on large-scale repos. For background on the pattern, see [IfChange/ThenChange](https://filiph.net/text/ifchange-thenchange.html), [Syncing Code](https://steve.dignam.xyz/2025/05/28/syncing-code/), and [Fuchsia presubmit checks](https://fuchsia.dev/fuchsia-src/development/source_code/presubmit_checks).
