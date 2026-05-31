# Codex Agent Evaluation

The final evaluation target is not only whether `workspace related` predicts
historical co-changes. The stronger claim is that a development agent such as
Codex can use `workspace-cli` to work more safely and efficiently in a real
workspace.

`tools/run_codex_workspace_pilot.py` is the first real Codex-in-the-loop pilot.
It creates the same temporary failing-test repository twice, then runs Codex
non-interactively in two conditions:

| condition | instruction |
| --- | --- |
| `shell_only` | Use ordinary shell tools and do not use `workspace`. |
| `workspace_cli` | Use `./bin/workspace` for status, search, read, diff, patch, run, and log-oriented work. |

Run it after building the workspace binary:

```sh
cargo build
python3 tools/run_codex_workspace_pilot.py \
  --output-dir target/codex-workspace-pilot
```

The pilot writes `summary.json`, `summary.md`, raw Codex JSONL, stderr logs,
command lists, final diffs, and the `workspace_cli` operation log. The summary
captures test success, elapsed seconds, command counts, workspace command
counts, workspace log entry counts, changed files, and the final diff.

## Current Pilot Result

The first locked-log pilot on this machine solved the task in both conditions:

| condition | passed | seconds | commands | workspace commands | workspace log entries | changed files |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `shell_only` | true | 62.937 | 12 | 0 | 0 | `src/checkout.py` |
| `workspace_cli` | true | 109.048 | 15 | 13 | 17 | `src/checkout.py` |

This tiny task is not evidence that `workspace-cli` is faster. It shows overhead:
`workspace_cli` took 46.111 seconds longer than `shell_only`. That is useful
negative evidence, and it means future agent-efficiency claims need larger tasks
where structured observation, transaction logs, impact analysis, and rollback
can pay for their overhead.

The pilot did produce one direct product improvement. A pre-fix run showed that
parallel Codex-issued `workspace read` operations could interleave writes to
`.workspace/log.jsonl`, making `workspace status` report `operation log
unreadable`. `append_operation_log` now takes an exclusive file lock before
writing each JSONL record. The locked-log pilot had 17 valid workspace log
entries and no `operation log unreadable` status.

## What This Proves

- Codex can be run non-interactively against controlled development tasks.
- Codex can be prompted to use `workspace-cli` for real observation,
  verification, and patch operations.
- The harness records enough evidence to compare success, overhead, command
  choice, final diffs, and workspace audit logs.
- Simple tasks are currently worse for `workspace-cli` on elapsed time, so the
  paper should not claim universal speedups from this pilot.

## Next Required Step

The next evaluation should use larger, repository-like tasks where the tool is
expected to help: multi-file edits, hidden related files, failing tests whose
source is not obvious from a single stack trace, impact checks after a patch,
and rollback from an intentionally bad first edit.
