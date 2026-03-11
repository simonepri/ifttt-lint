<p align="center">
  <a href="https://github.com/simonepri/ifttt-lint"><img src="assets/banner.png" alt="ifttt-lint" width="600" /></a>
</p>
<p align="center">
</p>
<p align="center">
  🔗 Enforce atomic cross-file changes via <code>LINT.IfChange</code> / <code>LINT.ThenChange</code> directives.
  <br/>
  <sub>
    Inspired by Google's internal IfThisThenThat linter, written in Rust, scales to large monorepos.
  </sub>
</p>
<p align="center">
  <a href="https://crates.io/crates/ifttt-lint"><img src="https://img.shields.io/crates/v/ifttt-lint.svg" alt="crates.io" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/simonepri/ifttt-lint.svg" alt="license" /></a>
</p>

## Synopsis

When you modify code inside an `IfChange` block, `ifttt-lint` verifies that every referenced `ThenChange` target was also modified in the same diff. If not, it fails — catching forgotten co-changes before they hit production.

## Usage

Add directives as comments in any language — the tool auto-detects comment styles based on file extension.

### Basic: If this file changes, that file must change too

```python
# LINT.IfChange
def time_travel(delorean, speed_mph):
    if speed_mph < 88:
        raise FluxError("insufficient speed")
    delorean.activate_flux_capacitor()
# LINT.ThenChange('//tests/test_time_travel.py')
```

### Labeled: Target a specific section

```python
# LINT.IfChange('flux_capacitor')
FLUX_CAPACITOR_WATTS = 1_210_000_000  # 1.21 gigawatts
# LINT.ThenChange('//docs/delorean.md:flux_capacitor')
```

```markdown
<!-- LINT.Label('flux_capacitor') -->
The flux capacitor requires exactly 1.21 gigawatts of power.
<!-- LINT.EndLabel -->
```

### Multiple targets

```python
# LINT.IfChange
SPEED_THRESHOLD_MPH = 88
# LINT.ThenChange([
#     '//db/migrations/1955_temporal_displacement.sql',
#     '//docs/delorean.md:speed_threshold',
#     ':time_circuits',
# ])
```

### Same-file references

```python
# LINT.IfChange('encode_coordinates')
def encode_time_coordinates(dest: datetime) -> bytes: ...
# LINT.ThenChange(':decode_coordinates')

# LINT.IfChange('decode_coordinates')
def decode_time_coordinates(data: bytes) -> datetime: ...
# LINT.ThenChange(':encode_coordinates')
```


## Setup

### pre-commit (recommended)

```yaml
# .pre-commit-config.yaml
- repo: https://github.com/simonepri/ifttt-lint
  rev: v0.1.0
  hooks:
    - id: ifttt-lint
```

### GitHub Actions

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- name: Lint cross-file changes
  run: |
    cargo install ifttt-lint
    ifttt-lint origin/${{ github.base_ref }}...HEAD
```

### Manual

```bash
cargo install ifttt-lint

ifttt-lint                    # auto-detect git upstream
ifttt-lint main...HEAD        # explicit ref range
ifttt-lint diff.patch         # from file
cat diff.patch | ifttt-lint - # from stdin
ifttt-lint --scan .           # scan for malformed directives
```

## Suppressing Findings

When you intentionally skip a `ThenChange` target, add `IFTTT_ALLOW=<target>` to your commit message:

```
feat: bump flux capacitor to 1.22 gigawatts

IFTTT_ALLOW=//docs/delorean.md:flux_capacitor
```

In git mode (`ifttt-lint main...HEAD` or auto-detect), the tool parses these tags from commit messages automatically.

For piped diffs, set the env var:

```bash
IFTTT_ALLOW="//docs/delorean.md:flux_capacitor" cat diff.patch | ifttt-lint -
```

To permanently ignore targets, use `--ignore`:

```bash
ifttt-lint --ignore "generated/*" --ignore "*.lock"
```

## Directive Reference

| Directive | Description |
|---|---|
| `LINT.IfChange` | Marks the start of a watched region |
| `LINT.IfChange('label')` | Watched region with a named label (targetable from other files) |
| `LINT.ThenChange('//path')` | End of watched region; requires target file to be modified |
| `LINT.ThenChange('//path:label')` | Requires changes within a specific label range in the target |
| `LINT.ThenChange(':label')` | Same-file label reference |
| `LINT.ThenChange([...])` | Multiple targets (array syntax) |
| `LINT.Label('name')` | Start of a named region (standalone, outside IfChange blocks) |
| `LINT.EndLabel` | End of a named region |

### Path Rules

- All file paths **must** start with `//` (project-root-relative)
- `:` separates file path from label (splits on last `:`)
- `:label` alone means same-file reference

### Label Format

Labels must start with a letter, followed by letters, digits, underscores, or dashes. For example: `my_label`, `flux-capacitor`, `section2`.

## CLI Reference

```
ifttt-lint [OPTIONS] [INPUT]
```

| Argument | Description |
|---|---|
| `(none)` | Auto-detect: git upstream if TTY, stdin if piped |
| `BASE...HEAD` | Git ref range — runs `git diff` and parses `IFTTT_ALLOW` from commits |
| `file.patch` | Read diff from file |
| `-` | Read diff from stdin |

| Option | Description |
|---|---|
| `--root <PATH>` | Project root for `//` paths. Defaults to git repo root or cwd |
| `-t, --threads <N>` | Worker thread count (0 = auto, default) |
| `-i, --ignore <PATTERN>` | Permanently ignore target pattern, repeatable (glob syntax) |
| `-s, --scan <DIR>` | Scan mode: validate directive syntax in a directory |
| `-f, --format <FMT>` | Output format: `pretty` (default), `json`, `plain` |

| Environment Variable | Description |
|---|---|
| `IFTTT_ALLOW` | Space-separated targets to suppress (glob syntax) |

| Exit Code | Meaning |
|---|---|
| `0` | No errors |
| `1` | Lint errors found |
| `2` | Fatal error (bad diff, I/O failure) |

## Supported Languages

Comment style is detected by file extension. Coverage includes **80+ extensions** across these style groups:

| Style | Extensions (sample) |
|---|---|
| `//` `/*` | `.js`, `.ts`, `.go`, `.rs`, `.java`, `.c`, `.cpp`, `.swift`, `.kt`, `.dart` |
| `#` | `.py`, `.rb`, `.sh`, `.yaml`, `.toml`, `.dockerfile`, `.tf`, `.nix` |
| `<!--` | `.html`, `.xml`, `.svg`, `.md`, `.mdx`, `.jsp` |
| `--` | `.sql`, `.lua`, `.hs`, `.ada`, `.vhdl` |
| `;;` | `.lisp`, `.clj`, `.scm`, `.rkt` |
| `%` | `.tex`, `.erl`, `.m` |
| `!` | `.f90`, `.f95`, `.for` |

Multi-style: `.vue`, `.svelte` (`//`, `/*`, `<!--`), `.php` (`//`, `/*`, `#`).

Unknown extensions fall back to `//`, `/*`, `#`.

## Performance

`ifttt-lint` is designed to add negligible overhead to your CI pipeline. In **diff mode** (the default), only files touched by the PR are read and parsed — a typical run with a handful of changed files completes in under 10 ms. Even in **scan mode**, where every file in the repo is validated, it handles thousands of files and tens of thousands of directives in well under a second.

This is possible because the tool skips files that don't contain `LINT.` directives (a simple substring check), parses directive syntax with a compiled [PEG grammar](src/grammar.pest), parallelizes file I/O across cores with [rayon](https://crates.io/crates/rayon), and uses sorted line indices with binary search for efficient range-overlap queries during validation.

Benchmarks run on every CI push — see the [latest run](../../actions) for exact numbers, or run `cargo bench` locally.

## Pipeline

```
diff / git range
       │
       ▼
 ┌───────────┐  changes   ┌───────────┐  findings   ┌──────────┐
 │   parse   │ ─────────▶ │   check   │ ──────────▶ │  report  │
 └───────────┘            └─────┬─────┘             └──────────┘
      (1)                   (2) │                      (3)
                          reads & parses files
                          (changed + targets)
```

1. **Parse the diff** — extract which line numbers changed in each file.
2. **Check targets** — find `LINT.IfChange` / `LINT.ThenChange` blocks in changed files and verify every referenced file (or labeled region) was also modified.
3. **Report** — emit findings in `pretty`, `plain`, or `json` format.

## License

MIT
