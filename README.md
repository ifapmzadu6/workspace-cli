# workspace cli

[![CI](https://github.com/ifapmzadu6/workspace-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/ifapmzadu6/workspace-cli/actions/workflows/ci.yml)

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
Read-only observation commands still return their observation if log recording
is unavailable; mutation and verification commands require a writable log.

### Validation

The current quality gates are:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
python3 -m unittest discover -s tools -p 'test_*.py'
python3 tools/measure_effect.py > /tmp/workspace-effect.json
python3 tools/check_effect_thresholds.py /tmp/workspace-effect.json
```

The same gates, plus `python3 tools/measure_effect.py`, run in GitHub Actions
on pushes to `main` and pull requests. The threshold check consumes the JSON
report from `tools/measure_effect.py` and fails if the fixture effect drops
below the expected floor. For paper-style temporal holdout reports, add
`--require-holdout` to also enforce the dense cross-repo holdout thresholds,
including minimum AP deltas against static, activity, centrality, direct, and
PageRank baselines, repo-macro checks that prevent one repository from
dominating the aggregate, plus Holm-adjusted paired sign-flip p-value ceilings.
`tools/run_effect_artifacts.py --paper` runs measurement, threshold checking,
Markdown rendering, and headline JSON extraction into one reproducible artifact
directory:

```sh
python3 tools/run_effect_artifacts.py --paper --output-dir target/effect-paper
python3 tools/verify_effect_artifacts.py target/effect-paper
```

The `Paper Effect Artifacts` GitHub workflow runs the clean-machine version of
that flow on demand and weekly: it clones the public holdout remotes, writes a
local manifest, generates the artifact directory, verifies it, and uploads the
artifact bundle.

If the pinned holdout repositories are not already checked out at the manifest
paths, materialize them from the recorded remotes first and run the artifact
flow against the generated local manifest:

```sh
python3 tools/prepare_effect_holdouts.py tools/effect_paper_holdouts.json \
  --repo-root target/effect-repos \
  --output-manifest target/effect-repos/holdouts.local.json
python3 tools/run_effect_artifacts.py \
  --manifest target/effect-repos/holdouts.local.json \
  --output-dir target/effect-paper
python3 tools/verify_effect_artifacts.py target/effect-paper
```

In addition to unit tests, the repository has integration tests that run the
real `workspace` binary inside temporary workspaces. The tests cover
`map/read`, the co-change index, `related/impact`, and the
`patch/run/log/diff/rollback` transaction flow.

Effect measurement is documented in
[docs/effect-measurement.md](docs/effect-measurement.md) and implemented by
`tools/measure_effect.py`. It measures observation coverage, related-file
discovery across multiple history topologies, seed-specific path-locality
lexical-similarity, and content-similarity baselines, seed-agnostic
recent-activity and global-PageRank baselines,
temporal-holdout predictable-only slices, and audit/rollback evidence instead
of only checking that commands run. The rendered effect summary also reports
holdout dataset composition, skipped commit reasons, target-label
distributions, history-only oracle ceilings, and case-level win/loss
diagnostics for paper-style reproducibility checks. It also reports
oracle-normalized AP, oracle gaps, residual gap clusters, and
leave-one-repo-out hybrid weight selection when sweep weights are provided.
Paper holdout threshold checks gate case-weighted and repo-macro effect sizes,
oracle-normalized AP, plus the corrected paired significance of the key deltas.
The artifact runner also writes `result_summary.json`, a compact machine-readable
summary of the headline metrics, full weight sweep, best weight result, and
per-repository holdout results, oracle-normalized AP gaps, residual gap
clusters including predictable-only retargeted gaps, plus the leakage audit.
Paper artifact directories include copies of the local holdout manifest and
source holdout manifest when available.
`run_manifest.json` records the exact commands and SHA-256 checksums for each
generated artifact and copied manifest. `tools/verify_effect_artifacts.py`
checks that the artifact directory has all required files, parseable JSON
outputs, a passing threshold log, SHA-256 hashes that match the run manifest,
holdout manifest hashes that match `effect.json` metadata, a recomputed
threshold pass, a Markdown summary that re-renders from `effect.json`, and a
result summary that matches `effect.json`. The manifest also records the
verifier command for artifact consumers.
The fixed-ref cross-repo holdout set
used for paper-style reproduction, including the dense hybrid weight sweep grid,
is captured in `tools/effect_paper_holdouts.json`. `tools/prepare_effect_holdouts.py`
can clone or refresh those repositories from their recorded public HTTPS remotes
and write a local manifest for the artifact runner.
Effect reports include reproducibility metadata with the workspace commit,
dirty state, resampling counts, exact sign-flip p-value method, holdout
manifest hash, source manifest hash for prepared local manifests, pinned
repository refs and remote URLs, and a temporal leakage audit that checks each
training index head against the held-out commit's parent.

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
`workspace` still enforces `--max-results` and bounded sample commit evidence on
`related-cli` output before returning an observation.

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
workspace related src/config.rs --by cochange --rank hybrid
workspace related src/config.rs --by cochange --rank hybrid --hybrid-direct-weight 0.25
```

`workspace index status` reports whether the saved index exists and whether it
is fresh for the current Git `HEAD`. `workspace index cochange --json` returns a
bounded summary of the saved index; the full edge list is persisted under
`.workspace/index/cochange.json` for later related/impact queries.

`--use-index`, `--rank pagerank`, and `--rank hybrid` use the saved co-change
graph to propagate from seed files through the graph. This can surface files
that were not directly changed with the seed file, but are connected through
related history. Related PageRank applies a small path prior for close file
siblings. `hybrid` combines that PageRank reachability with a direct co-change
boost, preserving indirect discovery while improving temporal holdout ranking
quality. Use `--hybrid-direct-weight` with values from `0.0` to `1.0` for
ablation runs; the default is `0.9` for `related`.

`workspace impact --diff --by cochange` uses the current Git diff as seed files
and returns nearby files from history. This helps decide what to read next and
which tests or documents may need verification.
Large seed-file lists are bounded in the observation. The full seed count
remains available as `seed_file_count`, with omitted seeds reported separately.
When `--rank pagerank` is used for impact analysis, tests receive a small rank
boost and documentation receives a small down-weight so likely verification
targets stay ahead of direct documentation noise. `impact --rank hybrid`
also accepts `--hybrid-direct-weight`; its default direct weight is `0.05`.

```sh
workspace impact --diff --by cochange --json
workspace impact --diff --max-commits 500 --max-results 30
workspace impact --diff --by cochange --use-index
workspace impact --diff --by cochange --rank pagerank
workspace impact --diff --by cochange --rank hybrid
workspace impact --diff --by cochange --rank hybrid --hybrid-direct-weight 0.10
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

### `workspace map`

Builds a structural map of the workspace. Large list fields are bounded in JSON
output; omitted counts are reported under `data.omitted` and the observation is
marked as `truncated`.

### `workspace status`

Summarizes the current workspace state:

```text
branch
dirty files
untracked files
recent operations
running commands
```

Large dirty and untracked file lists are bounded in JSON output. The total
counts remain available as `dirty_file_count` and `untracked_file_count`, with
omitted counts reported separately.

### `workspace search`

Searches within the workspace. The initial implementation is ripgrep-based.
Long matching lines are bounded and counted in `truncated_match_texts`.
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

Large read results are bounded and marked as `truncated` so observations stay
small enough for review and follow-up reads.

Future versions can read by symbol or document section.

### `workspace diff`

Shows the current diff. Future versions should distinguish user-authored changes
from agent-authored changes.
Full patch, stat output, and changed-file lists are bounded and marked as
`truncated`; use `--summary` for a smaller file/stat-only observation.
The full changed-file count remains available as `file_count`, with omitted
files reported separately.

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
Large changed-file lists are bounded in patch and rollback observations. The
total count remains available as `file_count`, with omitted files reported
separately.

### `workspace run`

Runs a command and records stdout, stderr, exit code, and duration.
Large stdout/stderr captures are bounded and marked as `truncated`.

```sh
workspace run "cargo test"
workspace run "npm test"
```

A command that exits with a nonzero status is still recorded as an observation:
the JSON output and operation log contain the child process `exit_code`, while
the `workspace` command itself remains successful.

Future versions can infer relevant tests from changed files or symbols.

### `workspace log`

Shows operations performed against the workspace.
Log entry `scope` and `summary` fields are bounded so long commands or manual
descriptions do not make later status/log observations unwieldy.
When `--limit` omits older log lines, the observation is marked as `truncated`
and reports the omitted line count.

```sh
workspace log
workspace log --limit 10
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
