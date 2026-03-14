---
trigger: always_on
---

# Code Philosophy

Code style and development principles.

## Overview

**Tiered guidance**:

- **Principles** — must follow (ranked by priority)
- **Decisions** — should follow
- **Best Practices** — may follow

**Caveat**: These guidelines are not justification for large-scale refactoring. Apply them to new code and incremental changes.

---

## Principles

Ordered by priority. When principles conflict, higher-ranked wins.

### 1. Clarity

The code's purpose and rationale is clear to the reader.

- **Purpose**: What the code does — achieved through naming, structure, and comments
- **Rationale**: Why it works this way — essential for non-obvious decisions

Allow code to speak for itself. Add comments for the "why", not the "what".

### 2. Simplicity

Accomplish goals in the simplest way possible.

Code should be easy to read from top to bottom. Prefer simpler tools when equivalent in capability.

### 3. Concision

High signal-to-noise ratio.

Remove repetition, extraneous syntax, and unnecessary abstraction. But never sacrifice clarity for brevity.

### 4. Maintainability

Code can be easily maintained.

Avoid hiding critical details in easily-overlooked places. Make important things visible.

### 5. Consistency

Code that looks, feels, and behaves like similar code in the codebase.

**Hierarchy**: Language idioms > Codebase patterns > Team conventions > Personal preference

**Local consistency**: When this guide provides no specific direction, match surrounding code.

---

## Decisions

Ordered by code lifecycle: name → design → errors → document → test.

### Naming

#### Follow language naming conventions

Use the standard file and identifier naming conventions of the language (e.g., `snake_case` in Python/Rust, `camelCase` in JS/TS, `PascalCase` in Go exports). Don't invent your own.

#### Accurate and distinct names

Names must not mislead — don't call a `Map` a `list` or a partial result `result`. And if two names differ, the difference must be meaningful — `Product`, `ProductData`, and `ProductInfo` are three names for the same concept.

<details>
<summary>Example</summary>

```
// Don't
accounts_list: Map<String, Account>  // it's a Map, not a list
fn getAccount() / fn getAccountInfo() // indistinguishable

// Do
accounts: Map<String, Account>
fn getAccount() / fn getAccountPermissions()
```

</details>

#### Verbs for actions, nouns for accessors

Use verbs for functions that perform actions. Getters/accessors can drop the `get` prefix.

<details>
<summary>Example</summary>

```
// Don't
function userDeletion(id) {}
user.getUserName()

// Do
function deleteUser(id) {}
user.name()
```

</details>

#### Length proportional to scope

Short names for small scopes, descriptive names for large scopes. Frequently-used identifiers can be shorter. When choosing between equivalent names, prefer one that's distinctive enough to search for — avoid overly generic names (`data`, `result`, `value`) for things you'll need to find across the codebase.

<details>
<summary>Example</summary>

```
// Don't
export function v() {}
cfg = loadApplicationConfiguration()
let data = fetchOrders()

// Do
export function validateUserPermissions() {}
cfg = loadConfig()
let pending_orders = fetchOrders()
```

</details>

#### Avoid redundancy with context

Names should not repeat information clear from context.

<details>
<summary>Example</summary>

```
// Don't
user.getUserName()
config.configTimeout

// Do
user.name()
config.timeout
```

</details>

---

### Function Design

#### Single Responsibility

Each function does one thing well. If you need "and" to describe it, split it.

<details>
<summary>Example</summary>

```
// Don't
validateAndSaveUser()

// Do
validate(user)
save(user)
```

</details>

#### One level of abstraction

Don't mix high-level intent with low-level mechanics in the same function. Each function should read at a single level of the story.

<details>
<summary>Example</summary>

```
// Don't: mixes orchestration with byte-level detail
fn process_order(order) {
    authorize(order.user)
    let parts = order.payload.split(b"\r\n")
    db.execute("INSERT INTO orders ...")
}

// Do: each call at the same level
fn process_order(order) {
    authorize(order.user)
    let items = parse_payload(order.payload)
    save_order(items)
}
```

</details>

#### Command-Query Separation

A function should either **do something** (command) or **answer something** (query), not both.

<details>
<summary>Example</summary>

```
// Don't: mutates AND returns status
fn set_name(name) -> bool

// Do: separate the command from the query
fn set_name(name)
fn is_valid_name(name) -> bool
```

</details>

#### Pure functions preferred

Isolate side effects. Pure functions are easier to test and reason about. Use dependency injection for external resources.

<details>
<summary>Example</summary>

```
// Don't
function process(items) {
  items.push(globalConfig.default)
}

// Do
function process(items, defaultItem) {
  return [...items, defaultItem]
}

// Don't
function createOrder() { user = db.find(...) }
// Do
function createOrder(db) { user = db.find(...) }
```

</details>

#### Small functions

Keep functions small (~50 lines). Large functions are hard to test and reason about.

#### Options objects for complexity

Use an options argument (object, struct, dataclass, kwargs) when a function has many parameters (~4+) or optional configuration.

<details>
<summary>Example</summary>

```
// Don't
createUser(name, retryCount, email, timeout)
fetch(url, true, false, 5000, null)

// Do
createUser(name, email, options)
fetch(url, options)
```

</details>

#### Early returns

Return early for edge cases and errors to keep the happy path unindented.

<details>
<summary>Example</summary>

```
// Don't
if (user) {
  if (user.active) {
    if (user.permitted) {
      /* logic */
    }
  }
}

// Do
if (!user) return
if (!user.active) return
if (!user.permitted) return
/* logic */
```

</details>

---

### Modules

#### One primary concept per file

Each file should export/define one main thing. Helpers can coexist but should be subordinate.

#### Top-down ordering

Organize files like a newspaper: high-level functions at the top, implementation details below. Where the language allows, place callees below their callers and group related functions together.

<details>
<summary>Example</summary>

```
// Don't: reader has to jump around
//   parse_header        ← detail, defined before caller
//   process_request     ← entry point buried in the middle
//   validate
//   parse               ← caller above, callees split above and below
//   parse_body
//   save

// Do: reads top to bottom
//   process_request     ← entry point first
//     validate          ← called by process_request
//     parse             ← called by process_request
//       parse_header    ←   called by parse
//       parse_body      ←   called by parse
//     save              ← called by process_request
```

</details>

#### Prefer absolute imports

Use paths relative to the project root instead of deep relative paths (`../../..`).

---

### Type Design

#### Protocol-first type families

When creating a family of related types (enum variants, interface implementations, subclasses), define the shared interface before implementing individual variants. Bottom-up construction — building one variant, then another, then retrofitting a common shape — leads to ad-hoc structure and special cases.

<details>
<summary>Example</summary>

```
// Don't — each variant invented independently
CardProcessor   { charge(amount, token) }
WalletProcessor { pay(amount, email) }
BankProcessor   { send(amount, routing) }
// consumers need per-type branching for every call site

// Do — shared protocol first, then implement
interface PaymentProcessor { process(amount, credentials) }
class CardProcessor   implements PaymentProcessor { ... }
class WalletProcessor implements PaymentProcessor { ... }
class BankProcessor   implements PaymentProcessor { ... }
```

</details>

#### Uniform variant structure

When a set of types share a role, prefer uniform structure so consumers don't need conditional logic. If most variants follow a pattern, make all of them follow it — special cases in producers create branching in consumers.

<details>
<summary>Example</summary>

```
// Don't — most variants are polymorphic, one is special-cased
exporter.export(report)                    // PDF, HTML
if (type == "csv") writeCsv(report)        // "too simple" for a class

// Do — every variant follows the same interface
exporter.export(report)  // PDF, HTML, and CSV all implement Exporter
```

</details>

---

### Error Handling

#### Explicit at boundaries, propagate internally

Handle errors explicitly at system boundaries (APIs, external services, user input). Let internal errors propagate.

#### Add context when wrapping

Add useful context when wrapping errors. Avoid redundant information already in the underlying error.

<details>
<summary>Example</summary>

```
// Don't: redundant
throw Error("Error: " + error.message)

// Do: add context
throw Error("Failed to create user: " + error.message)
```

</details>

#### No in-band error signals

Don't use special values (-1, null, empty string) to signal errors. Use explicit types, optionals, or exceptions.

#### Structured errors for programmatic handling

Use typed/structured errors to allow callers to handle errors programmatically rather than via string matching.

<details>
<summary>Example</summary>

```
// Don't
if error.message.contains("duplicate")
// Do
if error is DuplicateUserError
```

</details>

---

### Documentation

#### Contract not implementation

Document what the function does, not how it does it.

<details>
<summary>Example</summary>

```
// Don't
"Iterates through array and checks each element"
// Do
"Returns true if any element matches the predicate"
```

</details>

#### Document non-obvious parameters only

Document only error-prone or non-obvious parameters. Explaining every parameter adds noise.

#### Concurrency semantics when non-obvious

Document thread-safety and async behavior when it's not obvious from the signature.

#### TODO/FIXME/HACK convention

Use standard comment prefixes: **TODO** (planned improvement) · **FIXME** (known bug) · **HACK** (workaround to revisit). Include your name or a ticket reference.

---

### Testing

#### Test behavior, not implementation

Tests should verify what the code does, not how it does it. Implementation changes shouldn't break tests.

<details>
<summary>Example</summary>

```
// Don't
expect(service.cache.has("key")).toBe(true)
// Do
expect(service.get("key")).toBe("value")
```

</details>

#### Table-driven tests for similar cases

When testing multiple similar cases, use a table/parameterized structure.

#### Helpers for setup, assertions in test body

Test helpers do setup/cleanup. Assertions belong in the test function — never hide them in helpers.

#### Coverage: quality over quantity

Cover critical paths and edge cases, not arbitrary percentage targets.

---

## Best Practices

### Type Safety

#### Avoid dynamic typing

Avoid types that bypass the type checker (`any`, `Any`, `void*`, missing type hints). Use specific types, generics, or unknown-like types.

#### Parse, don't validate

Convert raw input to typed structures at system boundaries. Work with typed data internally.

<details>
<summary>Example</summary>

```
// Don't
passRawJsonThrough(rawJson)
validateAtEachStep(rawJson)

// Do
user = parseUser(rawJson)   // throws/errors if invalid
processUser(user)            // user is guaranteed valid
```

</details>

#### Explicit null/undefined handling

Be intentional about optionality. Don't use optional chaining or null-coalescing to silently ignore unexpected nulls.

---

### Data & State

#### Immutability

Prefer immutable data structures. Mutation makes code harder to reason about and introduces subtle bugs, especially in concurrent or async code. Create new objects instead of modifying existing ones.

#### Narrow visibility

Default to the narrowest visibility — only expose what consumers actually need. Don't reach through objects; talk to direct collaborators, not their internals.

<details>
<summary>Example</summary>

```
// Don't: reaches through the entire chain
order.customer().address().city()

// Do: ask for what you need
order.shipping_city()
```

</details>

---

### Architecture

#### Abstract at boundaries, not internally

Place interfaces at system boundaries (I/O, external services, plugin points) where you need substitutability for testing or multiple backends. Use concrete types and functions internally — internal interfaces add indirection without testability or extensibility benefit.

<details>
<summary>Example</summary>

```
// Don't — interfaces for internal modules nobody swaps out
interface Parser    { parse(content): AST }
interface Formatter { format(result): string }

// Do — interface at the I/O boundary, concrete internally
interface Storage { read(key): bytes; write(key, bytes) }
function parse(content): AST { ... }      // concrete
function format(result): string { ... }   // concrete
```

</details>

---

### Code Evolution

#### Rule of Three

Don't abstract until you have three concrete use cases. Two similar-looking cases often diverge later, making premature abstractions costly to undo.

#### Co-dependent changes: use IFTTT directives

When code in one place must stay in sync with code elsewhere — but DRY can't eliminate the duplication (e.g., cross-language boundaries, config mirroring schema, encode/decode pairs) — mark the dependency with `LINT.IfChange` / `LINT.ThenChange` directives so changes to one side force review of the other. Add these directives proactively when _creating_ new co-dependent content, not just when maintaining existing pairs.

<details>
<summary>Example</summary>

```
// LINT.IfChange(speed_threshold)
SPEED_THRESHOLD_MPH = 88
// LINT.ThenChange(
//     //db/migrations/temporal_displacement.sql,
//     //docs/delorean.md:speed_threshold,
// )
```

</details>

#### Don't re-implement stdlib

Before writing utility code, check if the language or a well-maintained library already solves it.
