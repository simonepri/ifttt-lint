<p align="center">
  <a href="https://github.com/simonepri/ifttt-lint"><img src="assets/banner.png" alt="ifttt-lint — LINT.IfChange / LINT.ThenChange linter for cross-file change enforcement" width="600" /></a>
</p>
<p align="center">
</p>
<p align="center">
  🔗 IfThisThenThat linter — enforce atomic cross-file changes via <code>LINT.IfChange</code> / <code>LINT.ThenChange</code> directives.
  <br/>
  <sub>
    Open Source implementation of <a href="https://www.chromium.org/chromium-os/developer-library/guides/development/keep-files-in-sync/">Google's internal IfThisThenThat linter</a>, written in Rust, scales to large monorepos.
  </sub>
</p>
<p align="center">
  <a href="https://crates.io/crates/ifttt-lint"><img src="https://img.shields.io/crates/v/ifttt-lint.svg" alt="crates.io" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/simonepri/ifttt-lint.svg" alt="license" /></a>
  <a href="https://codspeed.io/simonepri/ifttt-lint?utm_source=badge"><img src="https://img.shields.io/endpoint?url=https://codspeed.io/badge.json" alt="CodSpeed"/></a>
</p>

## Synopsis

You add a field to a Go struct and forget the TypeScript mirror. You bump a constant and forget the docs. You rename a database column and forget the migration. You only discover it when something breaks in production — or worse, when a user reports it weeks later.

Code review doesn't catch it: reviewers look at the diff, not the five other files that should have changed too. Static types don't help: the duplication crosses language boundaries. Tests are flaky, expensive, and only cover the cases you thought of. AI code review still misses non-trivial cross-file inconsistencies.

`ifttt-lint` catches it. You wrap co-dependent sections in `LINT.IfChange` / `LINT.ThenChange` comment directives — the IfChangeThenChange pattern. When a diff touches one side but not the other, the tool fails — before the change reaches production. It's stupidly simple, and that's why it works.

It is not a replacement for DRY — it is a safety net for the cases where DRY can't help, when duplication crosses language, directory, or system boundaries. For example this repo [dogfoods ittt directives](#used-in-this-repo) to keep the tool version in sync across [`Cargo.toml`](Cargo.toml), the [pre-commit config](#pre-commit-recommended), and the [CI release pipeline](.github/workflows/ci-cd.yml).

The pattern is not new — [Google's internal IfThisThenThat linter](https://www.chromium.org/chromium-os/developer-library/guides/development/keep-files-in-sync/) has enforced it across Chromium, TensorFlow, Fuchsia and virtually any other Google project for over a decade. It's the kind of tool you (hopefully) use once a quarter but miss every day it's gone. As an engineer at Google I took it for granted; when I left, nothing outside implemented this.

<sub>1. [if-changed](https://github.com/mathematic-inc/if-changed), [ifttt-lint](https://github.com/ebrevdo/ifttt-lint), and [ifchange](https://github.com/slnc/ifchange), exist but use different syntax and aren't [validated on large-scale repos](#performance). For background on the pattern see [IfChange/ThenChange](https://filiph.net/text/ifchange-thenchange.html), [Syncing Code](https://steve.dignam.xyz/2025/05/28/syncing-code/), and [Fuchsia presubmit checks](https://fuchsia.dev/fuchsia-src/development/source_code/presubmit_checks).</sub>

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

### Used in this repo

This repository dogfoods its own directives to keep the crate version in sync across [`Cargo.toml`](Cargo.toml), the [pre-commit config](#pre-commit-recommended) in this file, and the [CI release pipeline](.github/workflows/ci-cd.yml). If any of the three drifts, the linter catches it before merge.

## Setup

### pre-commit (recommended)

<!-- LINT.IfChange(version) -->

```yaml
# .pre-commit-config.yaml
- repo: https://github.com/simonepri/ifttt-lint
  rev: v0.5.0
  hooks:
    - id: ifttt-lint
```

<!-- LINT.ThenChange(//Cargo.toml:version, //.github/workflows/ci-cd.yml:version) -->

> [!NOTE]
> The hook validates the diff against your git upstream and also runs a structural validity check on every staged file, confirming that all `ThenChange` targets and labels exist on disk.

### GitHub Actions

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- name: Lint cross-file changes
  run: |
    cargo install ifttt-lint
    ifttt-lint --diff origin/${{ github.base_ref }}...HEAD
```

> [!TIP]
> `fetch-depth: 0` is required so git has the full history to compute the diff.

### Manual

```bash
cargo install ifttt-lint

ifttt-lint                              # auto-detect git upstream
ifttt-lint --diff main...HEAD           # explicit ref range
ifttt-lint src/**/*.rs                  # structural-only (no diff)
ifttt-lint --diff main...HEAD src/**/*.rs  # diff + structural
```

## CLI Reference

```
ifttt-lint [OPTIONS] [FILES]...
```

| Argument   | Description                                                                                                                                  |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `FILES...` | Files to validate structurally: checks that every `ThenChange` target and label exists on disk, regardless of whether the file was modified. |

| Option                   | Description                                                                                                                                      |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `-d, --diff <RANGE>`     | Git ref range to diff (e.g. `main...HEAD`). Default: auto-detect git upstream (TTY) or staged changes (piped, for pre-commit hooks)              |
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
> The tool parses `NO_IFTTT` tags from commit messages automatically and suppresses diff-based findings for that range. Structural validity checks (from `[FILES]...`) and reverse-lookup for deleted targets still run.

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

Use `--strict=false` for Google-compatible behavior — bare paths (`path/to/file`), single-`/` paths (`/path/to/file`), explicit same-file path references (`//same-file.h:label` instead of `:label`), and single-`/` paths are all accepted without warnings.

### Label Format

Labels must start with a letter, followed by letters, digits, underscores, dashes, or dots. For example: `upload_limit`, `user-response`, `section2`, `Payments.Pix.Result`.

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

## Performance

`ifttt-lint` is designed to add negligible overhead to your CI pipeline. In **diff mode** (the default), only files touched by the PR are read and parsed. The optional **structural validity pass** (triggered by `[FILES]...`) validates that all targets and labels referenced in the listed files exist on disk, reading only the files actually passed — no full-repo scan required.

When the diff **deletes** (or renames) a file, a **reverse-lookup pass** walks the repo to find surviving files that still reference the deleted target. This is the only scenario that triggers a repo-wide scan. The walk runs once (not once per deleted file) and uses two cheap substring filters to skip the vast majority of files before parsing: first, does the file contain `LINT.`? If not, skip. Then, does it mention any of the deleted paths? If not, skip. Files already parsed in earlier passes reuse their cached result. The walk uses `git grep` to discover candidates, which only searches tracked files and respects `.gitignore`. Untracked files containing LINT directives that reference a deleted target will not be caught — commit or stage them first.

This is possible because the tool skips files that don't contain `LINT.` directives (a simple substring check), parses directive syntax with a [data-driven scanner](src/languages.rs), parallelizes file I/O across cores with [rayon](https://crates.io/crates/rayon), and uses sorted line indices with binary search for efficient range-overlap queries during validation.

**Real-world** (structural validation, M-series MacBook):

| Repository                                             | Tracked files | Files with directives | 1 thread | 2 threads        | 4 threads    |
| ------------------------------------------------------ | ------------: | --------------------: | -------- | ---------------- | ------------ |
| [Chromium](https://github.com/chromium/chromium)       | 488k (~4.2GB) |          1.7k (~41MB) | 1.9 s    | **0.9 s** (2.0×) | 1.2 s (1.6×) |
| [TensorFlow](https://github.com/tensorflow/tensorflow) |  36k (~402MB) |          245 (~5.7MB) | 0.3 s    | **0.2 s** (1.3×) | 0.3 s (1.3×) |

<sub>Structural validation on M-series MacBook. Reproduce with `cargo smoke`.</sub>

The default thread count is **2** (`--threads 0`), which gives near-optimal throughput. Higher counts hit filesystem I/O contention and degrade.

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
