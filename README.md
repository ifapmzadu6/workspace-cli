# workspace cli

`workspace cli` is a CLI runtime for humans and LLM agents to read, change,
verify, and track a project workspace through the same operation surface.

The goal is not to build an AI coding assistant directly. The first useful
layer is a runtime that lets agents handle a workspace safely and efficiently.

```text
workspace_before + intent
        |
        v
workspace operations
        |
        v
workspace_after + evidence
```

## Current MVP

This repository contains a Rust CLI. The binary name is `workspace`.

### Installation

During development, run commands with `cargo run -- <command>`. To install the
local binary as `workspace`, use:

```sh
cargo install --path .
workspace --version
workspace --help
```

### Basic Commands

```sh
cargo run -- map
cargo run -- map --json
cargo run -- status
cargo run -- search "WorkspaceObservation"
cargo run -- index status
cargo run -- index cochange
cargo run -- related src/main.rs --by cochange
cargo run -- impact --diff --by cochange
cargo run -- read README.md --lines 1:40
cargo run -- diff --summary
cargo run -- run "cargo test"
cargo run -- log
```

The current command set provides:

```text
workspace map       Build a map of the workspace
workspace status    Show git state, index state, and recent operations
workspace search    Search with ripgrep
workspace index     Build workspace indexes
workspace related   Find files related by Git co-change history
workspace impact    Find likely impacted files from the current diff
workspace read      Read a text file or line range
workspace diff      Show the current git diff
workspace patch     Apply a patch as a recorded transaction
workspace run       Run a command and record the result
workspace log       Show the operation log
workspace rollback  Roll back a workspace patch transaction
```

Observation commands support `--json` and return structured output with
`summary`, `data`, `evidence`, and `next_observations` so an LLM agent can use
the result directly. Operation logs are stored in `.workspace/log.jsonl`.

### Validation

The current quality gates are:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```

In addition to unit tests, the repository has integration tests that run the
real `workspace` binary inside temporary workspaces. The tests cover
`map/read`, the co-change index, `related/impact`, and the
`patch/run/log/diff/rollback` transaction flow.

Effect measurement is documented in
[docs/effect-measurement.md](docs/effect-measurement.md) and implemented by
`tools/measure_effect.py`. It measures observation coverage, related-file
discovery, and audit/rollback evidence instead of only checking that commands
run.

### History-Based Related-File Observation

`workspace related <file> --by cochange` estimates related files from Git
history, not from file contents. Files changed in the same commit are treated as
operationally related. Recent commits are weighted more strongly, and broad
commits are down-weighted or filtered.

The intent is to answer:

```text
If I touch this file, which files did past work usually touch with it?
```

```sh
workspace related src/config.rs --by cochange --json
workspace related src/config.rs --max-commits 500 --max-files-per-commit 30
```

If a `related` binary is available, direct on-demand co-change observation uses
[related-cli](https://github.com/ifapmzadu6/related-cli) first. You can specify
it explicitly:

```sh
WORKSPACE_RELATED_BIN=/path/to/related workspace related src/config.rs --by cochange --json
WORKSPACE_RELATED_BIN=/path/to/related workspace impact --diff --by cochange --json
```

Set `WORKSPACE_RELATED_DISABLE=1` to force the internal implementation.
Set `WORKSPACE_RELATED_HISTORY_BACKEND` to pass a different
`related-cli --history-backend` value. When unset, `workspace` uses `git` for
exact history semantics.

Large formatting commits, dependency updates, lockfile churn, and initial import
commits can add noise. Use `--max-files-per-commit` to exclude broad commits.

For larger repositories, you can first persist a co-change graph at
`.workspace/index/cochange.json`.

```sh
workspace index status
workspace index cochange
workspace index cochange --max-commits 2000 --max-files-per-commit 30 --json
workspace related src/config.rs --by cochange --use-index
workspace related src/config.rs --by cochange --rank pagerank
```

`workspace index status` reports whether the saved index exists and whether it
is fresh for the current Git `HEAD`.

`--use-index` and `--rank pagerank` use the saved co-change graph to propagate
from seed files through the graph. This can surface files that were not directly
changed with the seed file, but are connected through related history.

`workspace impact --diff --by cochange` uses the current Git diff as seed files
and returns nearby files from history. This helps decide what to read next and
which tests or documents may need verification.

```sh
workspace impact --diff --by cochange --json
workspace impact --diff --max-commits 500 --max-results 30
workspace impact --diff --by cochange --use-index
workspace impact --diff --by cochange --rank pagerank
```

## Background

A bare LLM interaction is close to:

```text
input:  prompt
output: text
```

A development agent such as Codex is stronger because it also has a workspace.
It can read files, search, edit, run commands, observe test results, inspect
diffs, and roll back work when needed.

```text
input:
  prompt
  workspace state
  files
  git diff
  command output
  test results

output:
  text
  patches
  file edits
  commands
  verified state changes
```

That difference matters. With the same model, better workspace operations can
make practical performance better. Faster and more accurate exploration,
editing, verification, and rollback turn model reasoning into real work more
reliably.

## Concept

`workspace cli` treats a workspace as a stateful working environment, not just a
file tree.

```text
Workspace =
  filesystem
  git state
  process execution
  test outputs
  dependency graph
  symbol index
  documents
  operation log
```

Agents should not have to manipulate raw shell and filesystem operations without
structure. They should use explicit workspace operations:

```text
read
search
diff
patch
run
log
rollback
```

The result is a shared interface that is useful to both humans and agents.

## Design Principles

- Keep the CLI natural for humans, not only for AI agents.
- Prefer patch and transaction based edits.
- Attach diffs, operation logs, and verification results to changes.
- Separate observation from mutation.
- Start with local Git repositories.
- Build a smarter workspace operation surface before building a smarter agent.

## Minimal Command Model

```sh
workspace status
workspace search "ConfigToml"
workspace read README.md
workspace diff
workspace patch fix.patch
workspace run "just test -p codex-core"
workspace log
workspace rollback <change-id>
```

### `workspace status`

Summarizes the current workspace state:

```text
branch
dirty files
untracked files
recent operations
running commands
```

### `workspace search`

Searches within the workspace. The initial implementation is ripgrep-based.
Future versions can combine:

```text
text search
symbol search
semantic search
dependency search
```

### `workspace read`

Reads a file or range:

```sh
workspace read src/main.rs
workspace read src/main.rs --lines 40:120
```

Future versions can read by symbol or document section.

### `workspace diff`

Shows the current diff. Future versions should distinguish user-authored changes
from agent-authored changes.

```sh
workspace diff
workspace diff --summary
```

### `workspace patch`

Applies a patch as a transaction:

```sh
workspace patch fix.diff
```

Patch-first mutation is easier to review, track, and roll back than direct file
writes.

### `workspace run`

Runs a command and records stdout, stderr, exit code, and duration.

```sh
workspace run "cargo test"
workspace run "npm test"
```

Future versions can infer relevant tests from changed files or symbols.

### `workspace log`

Shows operations performed against the workspace.

```sh
workspace log
```

This is not agent memory. It is an auditable history stored on the workspace
side.

### `workspace rollback`

Rolls back a specific change. The goal is transaction-level rollback for changes
applied by `workspace cli`, not broad operations such as `git reset`.

## Abstract Model

Core concepts:

```rust
struct WorkspaceState {
    root: PathBuf,
    git: GitState,
    files: FileIndex,
    operations: Vec<OperationLog>,
}
```

A snapshot of workspace state.

```rust
enum WorkspaceOperation {
    Read(Path),
    Search(Query),
    ApplyPatch(Patch),
    Run(Command),
    Rollback(TransactionId),
}
```

An operation against the workspace.

```rust
struct Observation {
    summary: String,
    evidence: Vec<Evidence>,
    next_observations: Vec<SuggestedAction>,
}
```

An observation produced by an operation.

```rust
struct Transaction {
    id: TransactionId,
    patch: Patch,
    verification: Vec<CommandResult>,
    created_at: DateTime,
}
```

A grouped change that can include a patch, generated files, related commands,
and verification results.

## Relationship To Agents

An LLM agent uses `workspace cli` as a tool:

```text
User intent
  -> agent chooses workspace operations
  -> workspace cli returns observations
  -> agent decides next operation
  -> patch/run/verify
  -> workspace_after + evidence
```

The agent should fetch only the observations it needs instead of loading every
file into context.

The goal is not to increase context size. The goal is to improve access
efficiency to the workspace.

## MVP Scope

The first MVP focuses on:

```text
status
search
read
diff
patch
run
log
rollback
```

Advanced symbol indexes and semantic search can come later.

The key MVP requirement is that the CLI is useful to a human before it is useful
to an AI.

## Future Extensions

### Symbol Index

Use Tree-sitter or language servers to expose functions, types, classes, and
reference relationships.

```sh
workspace symbols src/main.rs
workspace references WorkspaceState
```

### Document Index

Handle Markdown and docs by section.

```sh
workspace docs map
workspace docs read README.md#installation
```

### Test Selection

Infer which tests should run from changed files or symbols.

```sh
workspace tests suggest
workspace tests run-related
```

### MCP Server

Expose the same operation surface as an MCP server, not only as a CLI.

```text
workspace-cli
workspace-mcp
workspace-sdk
```

This would let Codex, Claude, Cursor, and other agents use the same workspace
operation surface.

### UI

Add a TUI or web UI that makes diffs, verification results, and rollback points
easy to inspect.

## Non-Goals

- Do not start by building a full AI coding agent.
- Do not start by building an IDE.
- Do not start with cloud sync.
- Do not start with advanced code intelligence for every language.
- Do not try to fully replace the raw shell.

## Open Questions

- What are the minimal primitives for workspace operations?
- How should patch transaction boundaries be chosen?
- How should user-authored changes and agent-authored changes be distinguished?
- How should the operation log relate to Git history?
- How should command execution balance safety and freedom?
- When should workspace indexes be refreshed?
- How should responsibilities be split across CLI, SDK, and MCP server?

## Implementation Notes

Rust and Go are good candidates for this kind of CLI, though TypeScript can work
for an MVP.

The data model matters more than the implementation language.

```text
workspace operation
  -> structured result
  -> evidence
  -> suggested next operation
```

The CLI should separate human-readable output from JSON output for agents.

```sh
workspace status
workspace status --json
```

## In One Sentence

`workspace cli` is an observable and reversible workspace operation surface for
LLM agents and humans.
