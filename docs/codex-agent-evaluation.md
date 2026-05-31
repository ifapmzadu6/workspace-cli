# Codex Agent Evaluation

The final evaluation target is not only whether `workspace related` predicts
historical co-changes. The stronger claim is that a development agent such as
Codex can use `workspace-cli` to work more safely and efficiently in a real
workspace.

`tools/run_codex_workspace_pilot.py` runs real Codex-in-the-loop pilots. For
each selected task, it creates the same temporary failing-test repository twice,
then runs Codex non-interactively in two conditions:

| condition | instruction |
| --- | --- |
| `shell_only` | Use ordinary shell tools and do not use `workspace`. |
| `workspace_cli` | Use `./bin/workspace` for status, search, read, diff, patch, run, and log-oriented work. |

Run it after building the workspace binary:

```sh
cargo build
python3 tools/run_codex_workspace_pilot.py \
  --output-dir target/codex-workspace-pilot
python3 tools/run_codex_workspace_pilot.py \
  --task policy_threshold_sync \
  --output-dir target/codex-workspace-pilot-policy
python3 tools/run_codex_workspace_pilot.py \
  --task rollback_recovery \
  --output-dir target/codex-workspace-pilot-rollback
```

The pilot writes `summary.json`, `summary.md`, raw Codex JSONL, stderr logs,
command lists, final diffs, and the `workspace_cli` operation log. The summary
captures test success, elapsed seconds, command counts, workspace command
counts, workspace log entry counts, changed files, and the final diff.
For rollback-oriented tasks it also records the number of `workspace rollback`
operations observed in the workspace operation log.

## Current Pilot Results

The first locked-log checkout pilot on this machine solved the task in both
conditions:

| condition | passed | seconds | commands | workspace commands | workspace log entries | changed files |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `shell_only` | true | 62.937 | 12 | 0 | 0 | `src/checkout.py` |
| `workspace_cli` | true | 109.048 | 15 | 13 | 17 | `src/checkout.py` |

This tiny task is not evidence that `workspace-cli` is faster. It shows overhead:
`workspace_cli` took 46.111 seconds longer than `shell_only`. That is useful
negative evidence, and it means future agent-efficiency claims need larger tasks
where structured observation, transaction logs, impact analysis, and rollback
can pay for their overhead.

The first co-change-oriented policy pilot also solved the task in both
conditions:

| condition | passed | seconds | commands | workspace commands | workspace log entries | changed files |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `shell_only` | true | 57.656 | 13 | 0 | 0 | `config/discount_policy.json`, `docs/discount_policy.md` |
| `workspace_cli` | true | 103.745 | 13 | 12 | 16 | `config/discount_policy.json`, `docs/discount_policy.md` |

The `workspace_cli` run used `workspace index cochange`, `workspace related
tests/test_discounts.py --by cochange --use-index --rank hybrid`, `workspace
patch`, `workspace impact --diff --by cochange --use-index --rank hybrid`, and
`workspace diff`. It still took 46.089 seconds longer than `shell_only`, so this
pilot is evidence that the current tool protocol can guide Codex through
co-change and impact-aware work, not evidence of an elapsed-time win.

The first rollback-oriented pilot solved the task in both conditions:

| condition | passed | seconds | commands | workspace commands | workspace log entries | rollback ops | changed files |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `shell_only` | true | 101.189 | 18 | 0 | 0 | 0 | `docs/billing.md`, `src/billing.py` |
| `workspace_cli` | true | 88.939 | 11 | 10 | 10 | 1 | `docs/billing.md`, `src/billing.py` |

In this single run, `workspace_cli` finished 12.250 seconds faster than
`shell_only`. The important qualitative result is that Codex used the intended
transactional workflow: `workspace patch` applied the intentionally bad proposed
patch, `workspace run` captured the failing test, `workspace rollback` reverted
that transaction, and a second `workspace patch` applied the correct late-fee
cap fix. This is the first positive timing pilot, but it is still only one run
on one controlled task.

The pilot did produce one direct product improvement. A pre-fix run showed that
parallel Codex-issued `workspace read` operations could interleave writes to
`.workspace/log.jsonl`, making `workspace status` report `operation log
unreadable`. `append_operation_log` now takes an exclusive file lock before
writing each JSONL record. The locked-log pilot had 17 valid workspace log
entries and no `operation log unreadable` status.

## What This Proves

- Codex can be run non-interactively against controlled development tasks.
- Codex can be prompted to use `workspace-cli` for real observation,
  verification, patch, rollback, related-file, and impact operations.
- The harness records enough evidence to compare success, overhead, command
  choice, final diffs, and workspace audit logs.
- The timing evidence is mixed: simple checkout and co-change tasks were slower
  with `workspace-cli`, while the rollback task was faster in one run. The paper
  should not claim universal speedups from these pilots.

## Next Required Step

The next evaluation should repeat these Codex pilots across multiple seeds and
larger repository-like tasks, then report pass rate, elapsed time, command
counts, rollback usage, and final diff correctness with confidence intervals or
at least bootstrap intervals.
