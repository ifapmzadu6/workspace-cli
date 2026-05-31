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
python3 tools/run_codex_workspace_suite.py \
  --tasks rollback_recovery \
  --repetitions 2 \
  --output-dir target/codex-workspace-suite-rollback
python3 tools/run_codex_workspace_suite.py \
  --tasks invoice_tax_sync \
  --repetitions 2 \
  --output-dir target/codex-workspace-suite-invoice-tax
python3 tools/run_codex_workspace_suite.py \
  --tasks policy_threshold_sync invoice_tax_sync rollback_recovery \
  --repetitions 2 \
  --output-dir target/codex-workspace-suite-cross-task
python3 tools/run_codex_workspace_suite.py \
  --tasks policy_threshold_sync invoice_tax_sync rollback_recovery subscription_rollout_sync \
  --repetitions 2 \
  --output-dir target/codex-workspace-suite-cross-task-subscription
python3 tools/run_codex_workspace_suite.py \
  --tasks subscription_rollout_sync \
  --repetitions 4 \
  --output-dir target/codex-workspace-suite-subscription-rollout
python3 tools/run_codex_workspace_suite.py \
  --merge-suite \
  target/codex-workspace-suite-policy \
  target/codex-workspace-suite-invoice-tax \
  target/codex-workspace-suite-rollback \
  --output-dir target/codex-workspace-suite-merged
```

The pilot writes `summary.json`, `summary.md`, raw Codex JSONL, stderr logs,
command lists, final diffs, and the `workspace_cli` operation log. The summary
captures test success, elapsed seconds, command counts, workspace command
counts, workspace log entry counts, changed files, and the final diff.
For rollback-oriented tasks it also records the number of `workspace rollback`
operations observed in the workspace operation log.

The suite runner repeats one or more pilot tasks and writes `suite_summary.json`
and `suite_summary.md`, while preserving each per-run pilot artifact directory.
It reports pass rate, expected-diff-scope correctness, elapsed-time bootstrap
intervals, command counts, workspace log usage, rollback usage, paired
`workspace_cli - shell_only` timing deltas, and paired command-count deltas. It
can also merge existing suite directories into a larger aggregate summary without
rerunning Codex, preserving the source suite metadata in `source_suites`.

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

After the same protocol improvements used for the invoice task
(`related --ensure-index --include-content` and `patch --stdin`), the
co-change-oriented policy task improved substantially. The optimized four-run
suite was:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 1.000 | 38.630 (33.642, 44.239) | 6.250 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 36.876 (33.203, 40.399) | 5.000 | 5.000 | 5.000 | 0.000 |

The paired timing delta was `workspace_cli - shell_only = -1.754s` with a
bootstrap interval of `(-10.914, 6.756)`. `workspace_cli` was faster in two
paired runs and `shell_only` was faster in two. This smaller task no longer
shows the large overhead of the initial policy pilot, but the elapsed-time
result remains neutral rather than statistically supported.

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

That positive timing result did not survive a small repeated suite after
removing Python bytecode-cache cleanup noise from the test command. The fixture
test command is now
`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests`, so Codex
does not spend extra commands deleting `__pycache__`
directories. The bytecode-off rollback suite ran two paired repetitions:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 2 | 1.000 | 1.000 | 62.721 (55.061, 70.381) | 10.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 2 | 1.000 | 1.000 | 114.823 (114.110, 115.537) | 13.000 | 10.500 | 10.000 | 1.000 |

The paired timing delta was `workspace_cli - shell_only = +52.103s` with a
bootstrap interval of `(45.156, 59.049)` over these two runs, and `shell_only`
was faster in both paired runs. Both conditions still passed every run and
touched exactly the expected files. `workspace_cli` also used rollback exactly
once per run, so the repeated result supports the auditability and recovery
claim but not a speedup claim for this small task.

The command logs showed that `workspace_cli` spent avoidable turns on
`workspace --help`, command-specific help, and `.workspace` metadata inspection.
After tightening the workspace prompt to state that the command syntax was
complete and that help/metadata inspection should be skipped, the same two-run
rollback suite improved:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 2 | 1.000 | 1.000 | 66.729 (64.549, 68.909) | 11.000 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 2 | 1.000 | 1.000 | 86.996 (72.233, 101.759) | 8.000 | 6.000 | 7.000 | 1.000 |

This prompt-level optimization reduced `workspace_cli` mean time by 27.827
seconds and mean command count from 13.000 to 8.000, while preserving pass rate,
diff-scope correctness, and rollback usage. The paired timing delta was still
positive at `+20.267s`, so the result is an overhead reduction, not a speedup.

After removing the installed `bin/workspace` binary from fixture Git history,
skipping the initial `workspace status`, reading only the proposed patch after
rollback, and applying the correct fix through `workspace patch --stdin`, the
rollback suite produced the first repeated positive timing result:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 1.000 | 61.097 (56.721, 66.228) | 11.750 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 51.947 (49.312, 55.840) | 6.000 | 6.000 | 7.000 | 1.000 |

The paired timing delta was `workspace_cli - shell_only = -9.150s` with a
bootstrap interval of `(-12.249, -5.653)`. `workspace_cli` was faster in all
four paired runs, used 5.750 fewer commands on average, preserved expected
diff scope, and used exactly one rollback per run. This is still a controlled
pilot task, but unlike the earlier timing results its interval does not cross
zero.

The first larger multi-file synchronization task, `invoice_tax_sync`, requires
Codex to update tax configuration, invoice-label configuration, and two
documentation files after an EU digital VAT policy change. The fixture includes
shipping and promotion decoys, and its history co-changes
`tests/test_invoice_pipeline.py` with the four target files. The workspace run
uses `workspace related tests/test_invoice_pipeline.py --by cochange
--ensure-index --max-commits 1000 --rank hybrid --include-content`,
`workspace patch --stdin`, `workspace run`,
`workspace impact --diff --ensure-index`, and `workspace diff`.

An initial single run solved the task in both conditions but showed avoidable
workspace overhead: `workspace_cli` took 122.310 seconds and 15 commands versus
66.634 seconds and 11 commands for `shell_only`. Tightening the workspace prompt
to state that `workspace patch` requires a standard unified git diff file, and
that `workspace run` should use the quoted-command form rather than a `--`
separator, reduced the single-run workspace result to 78.729 seconds and 12
commands. The pre-content two-run invoice suite still favored `shell_only` on
time:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 2 | 1.000 | 1.000 | 49.153 (48.234, 50.071) | 10.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 2 | 1.000 | 1.000 | 75.674 (72.056, 79.293) | 12.000 | 11.000 | 11.000 | 0.000 |

The paired timing delta was `workspace_cli - shell_only = +26.522s` with a
bootstrap interval of `(21.985, 31.059)`. This task is useful evidence that
co-change discovery can steer Codex to the correct multi-file scope with perfect
diff-scope correctness in the observed runs, but it still does not show an
elapsed-time win.

That run exposed a concrete product issue: after `workspace related` identified
the four target files, Codex spent four more tool calls reading them. The CLI now
supports `workspace related --include-content --max-content-files <N>`, which
includes bounded content for top related files in the same JSON observation. On
the same invoice task, a single run improved `workspace_cli` from 78.729 seconds
and 12 commands to 71.756 seconds and 7 commands while preserving the exact
four-file diff. The corresponding two-run suite was:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 2 | 1.000 | 1.000 | 58.120 (54.964, 61.276) | 11.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 2 | 1.000 | 1.000 | 66.894 (62.234, 71.553) | 7.500 | 7.000 | 7.000 | 0.000 |

The paired timing delta improved to `workspace_cli - shell_only = +8.773s` with
a bootstrap interval of `(7.270, 10.277)`, and `workspace_cli` used four fewer
commands on average than `shell_only`. This is a real overhead reduction and a
clearer agent-efficiency path, but it is still not an elapsed-time win.

The remaining avoidable overhead was the separate `workspace status` and
`workspace index cochange` setup before the related-file query. The CLI now
supports `--ensure-index` on `workspace related` and `workspace impact`, creating
or refreshing the co-change index inside the same observation. A single invoice
run with `related --ensure-index --include-content` improved `workspace_cli` to
57.373 seconds, 6 total commands, and 5 workspace commands. The four-run suite
then showed stable command-count savings and elapsed-time parity:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 1.000 | 64.912 (58.788, 73.880) | 11.750 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 65.351 (56.602, 72.290) | 6.250 | 5.000 | 5.000 | 0.000 |

The paired timing delta was `workspace_cli - shell_only = +0.439s` with a
bootstrap interval of `(-9.344, 10.223)`; `workspace_cli` was faster in two
paired runs and `shell_only` was faster in two. The elapsed-time result is
therefore neutral, not a statistically supported speedup. The command-count
result is stronger: `workspace_cli` used 5.5 fewer commands on average while
maintaining perfect pass rate and expected diff scope.

The final fixed overhead in this workflow was creating a temporary patch file,
running `workspace patch <patch-file>`, then deleting that file. The CLI now
supports `workspace patch --stdin`, which stores the stdin unified diff as the
transaction patch and applies it without a user-visible temporary patch file.
With `related --ensure-index --include-content` plus `patch --stdin`, the
four-run invoice suite was:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 1.000 | 57.236 (44.699, 69.773) | 10.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 47.878 (44.143, 50.774) | 5.000 | 5.000 | 5.000 | 0.000 |

The paired timing delta was `workspace_cli - shell_only = -9.358s` with a
bootstrap interval of `(-19.542, 1.188)`. `workspace_cli` was faster in three
paired runs and `shell_only` was faster in one. Workspace-assisted Codex
maintained perfect pass rate and expected multi-file diff scope, cut mean
command count by 5.5, and was faster on average. The interval still crosses
zero, so this remains pilot evidence rather than a statistically powered
elapsed-time claim.

After the fixture cleanup and protocol optimizations, a cross-task suite over
`policy_threshold_sync`, `invoice_tax_sync`, and `rollback_recovery` produced a
stronger aggregate result on the latest commit:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 6 | 1.000 | 1.000 | 50.509 (41.892, 58.606) | 9.833 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 6 | 1.000 | 1.000 | 42.439 (37.697, 46.349) | 5.333 | 5.333 | 5.667 | 0.333 |

Across the six paired passing runs, `workspace_cli - shell_only` was `-8.070s`
with a bootstrap interval of `(-14.308, -2.085)`. `workspace_cli` was faster in
five runs and `shell_only` was faster in one. The paired command-count delta was
`-4.500` commands with interval `(-6.500, -2.333)`. By task, the mean timing
deltas were `invoice_tax_sync = -5.189s`, `policy_threshold_sync = -1.927s`,
and `rollback_recovery = -17.094s`. This is still a small controlled suite, but
it is the first multi-task Codex-in-the-loop result where the aggregate timing
interval does not cross zero.

Merging the existing task-specific four-run suites for the same three optimized
tasks gives a 12-pair aggregate without discarding the original per-run
artifacts:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 12 | 1.000 | 1.000 | 52.321 (45.009, 59.710) | 9.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 12 | 1.000 | 1.000 | 45.567 (41.270, 49.767) | 5.333 | 5.333 | 5.667 | 0.333 |

The 12-pair timing delta was `-6.754s` with bootstrap interval
`(-11.528, -1.933)`, and `workspace_cli` was faster in nine of twelve paired
runs. The paired command-count delta was `-4.167` commands with interval
`(-5.500, -2.833)`. The merged source suites were generated from clean
workspaces at the optimized task-specific commits rather than one single commit,
so this should be cited as a merged evidence artifact; the same-commit two-run
cross-task suite above is the cleaner commit-local result.

The next optimization removed `workspace impact` from the small two-file policy
task and tightened the stdin-patch instruction to include complete JSON
closing-brace context. A fresh policy-only four-run suite on commit
`c61dd07df5bf` moved the policy task from elapsed-time parity to a small
workspace advantage: `workspace_cli - shell_only = -1.198s` with interval
`(-2.365, -0.031)`, while using 1.750 fewer commands on average.

With that policy prompt trim, the same current commit produced the strongest
same-commit cross-task result so far:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 12 | 1.000 | 1.000 | 46.652 (39.882, 52.605) | 9.000 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 12 | 1.000 | 1.000 | 40.726 (36.424, 45.373) | 5.000 | 5.000 | 5.333 | 0.333 |

Across the twelve paired passing runs, the timing delta was `-5.926s` with
bootstrap interval `(-9.277, -3.012)`. `workspace_cli` was faster in ten of
twelve pairs. The paired command-count delta was `-4.000` commands with
interval `(-5.250, -2.750)`. By task, the timing deltas were
`invoice_tax_sync = -10.733s`, `policy_threshold_sync = -0.365s`, and
`rollback_recovery = -6.679s`; invoice and rollback had non-zero-crossing
task-level intervals, while policy remained close to parity but used fewer
commands.

To test a larger repository-like workflow, the harness now includes
`subscription_rollout_sync`. This task synchronizes a six-file Enterprise Plus
rollout across plan limits, feature flags, billing catalog data, two
documentation files, and a welcome email template, with support/trial decoys in
the repository. The first single pilot solved the task in both conditions:
`shell_only` took 59.346 seconds and 13 commands, while `workspace_cli` took
50.810 seconds and 5 commands with the exact six-file diff.

The first repeated four-run subscription suite gave a mixed timing result but a
strong safety and command-count result:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 0.500 | 68.769 (61.742, 76.741) | 12.750 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 81.844 (66.233, 97.574) | 5.500 | 5.500 | 5.000 | 0.000 |

The paired timing delta was `+13.075s` with interval `(-10.508, 34.911)`, so
this first larger-task suite did not support a speedup claim. However,
`workspace_cli` used 7.250 fewer commands on average with interval
`(-10.006, -4.250)` and kept the final diff exactly scoped in every run.
`shell_only` passed every run but edited the support-policy decoy in two of four
runs. This is evidence that the related-file workflow can reduce over-editing on
larger tasks even when elapsed time remains dominated by Codex reasoning
variance.

The subscription protocol was then shortened to ask Codex to edit only the six
target files, avoid support/trial decoys, and finish with `workspace diff --json`
instead of an impact analysis. The follow-up four-run suite at commit
`968819fb0f2dfe137b2beaf9717fe37306cc37e2` improved both timing and command
count while preserving the safety result:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 0.750 | 71.058 (63.207, 82.372) | 15.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 62.432 (48.130, 76.735) | 4.500 | 4.500 | 4.250 | 0.000 |

The shortened-protocol paired timing delta was `-8.626s` with interval
`(-19.542, 12.027)`. `workspace_cli` was faster in three of four pairs, but the
interval still crosses zero, so this remains pilot evidence rather than a
standalone larger-task speedup claim. The command-count delta strengthened to
`-11.000` commands with interval `(-12.500, -9.500)`, and `workspace_cli` again
kept the exact six-file diff in every run while `shell_only` over-edited the
support-policy decoy in one run.

The same-commit cross-task suite that included the shortened subscription task
at commit `fc9ea0223b952828a1399a96ebade035dc81c7bf` completed eight paired
passing runs:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 8 | 1.000 | 1.000 | 54.706 (46.946, 62.143) | 11.000 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 8 | 1.000 | 1.000 | 54.496 (44.552, 65.055) | 4.875 | 4.875 | 5.000 | 0.250 |

The aggregate paired timing delta was `-0.210s` with interval
`(-5.252, 6.099)`, so adding the subscription task moves the same-commit
cross-task suite from a speedup claim to parity. `workspace_cli` was faster in
five of eight pairs, and the command-count delta remained strong at `-6.125`
commands with interval `(-8.125, -4.000)`. By task, invoice and rollback still
had non-zero-crossing timing wins (`-5.602s` and `-5.181s` respectively), policy
remained near parity (`-0.485s`), and subscription was slower in both pairs
(`+10.428s`, interval `(2.149, 18.706)`) while still using exactly ten fewer
commands per pair.

Trace inspection showed that subscription latency was not in the `workspace`
commands themselves. It came from Codex hand-writing a six-file unified diff and,
in one run, retrying after a corrupt patch hunk. The CLI now includes
`workspace replace --stdin --json`, which accepts exact string replacements,
generates a rollbackable transaction patch, and lets Codex avoid manual hunk
line accounting. The clean four-run subscription suite at commit
`8e7666559c5fddc1163cfc6c6ea36635a2d1ce8d` produced a non-zero-crossing timing
win:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 0.500 | 73.606 (61.254, 94.603) | 14.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 43.520 (40.223, 46.818) | 4.000 | 4.000 | 4.000 | 0.000 |

The paired timing delta was `-30.086s` with interval `(-52.706, -15.713)`;
`workspace_cli` was faster in all four pairs. The paired command-count delta was
`-10.500` commands with interval `(-16.500, -5.500)`. The same clean commit also
restored the four-task same-commit aggregate speedup:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 8 | 1.000 | 1.000 | 56.594 (52.532, 61.237) | 10.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 8 | 1.000 | 1.000 | 44.824 (39.874, 50.185) | 4.750 | 4.750 | 5.000 | 0.250 |

Across the eight paired passing runs, the timing delta was `-11.770s` with
interval `(-19.055, -4.033)`, and `workspace_cli` was faster in six of eight
pairs. The command-count delta was `-5.750` commands with interval
`(-8.000, -3.500)`. By task, the timing deltas were
`invoice_tax_sync = -15.326s`, `policy_threshold_sync = -12.382s`,
`rollback_recovery = +4.860s`, and `subscription_rollout_sync = -24.232s`.
Rollback was slower in this two-run aggregate, but it still used fewer commands,
and the aggregate interval did not cross zero.

Trace inspection showed that rollback latency came from a four-step validation
loop: apply the proposed patch, run tests, roll the patch back, then read the
bad patch before constructing the correct fix. The CLI now includes
`workspace trial`, which applies a patch, runs a verifier, rolls the patch back
on failure, and returns the verifier output plus bounded patch content in one
observation. A first real Codex run exposed an additional prompt/schema issue:
Codex tried `old`/`new` replacement fields, then spent three commands inspecting
`./bin/workspace`. Tightening the prompt to provide the exact `find`/`replace`
JSON schema removed that detour. The clean four-run rollback suite at commit
`28362d330cd0dd13064675c82750fe0ff60c8145` produced:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 4 | 1.000 | 1.000 | 61.258 (55.861, 68.512) | 11.250 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 4 | 1.000 | 1.000 | 33.816 (30.119, 37.057) | 4.000 | 4.000 | 4.000 | 1.000 |

The paired timing delta was `-27.443s` with interval `(-35.891, -18.992)`;
`workspace_cli` was faster in all four pairs. The paired command-count delta was
`-7.250` commands with interval `(-9.500, -5.494)`. This directly fixes the
rollback regression observed in the previous clean four-task aggregate.

Rerunning the four-task same-commit aggregate with `workspace trial` included at
commit `d60671ba693b6ce1a43d9e54944002b8315cf96c` produced:

| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean workspace log entries | mean rollback ops |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `shell_only` | 8 | 1.000 | 1.000 | 56.813 (45.400, 67.082) | 10.500 | 0.000 | 0.000 | 0.000 |
| `workspace_cli` | 8 | 1.000 | 1.000 | 39.851 (33.992, 47.486) | 4.375 | 4.375 | 4.250 | 0.250 |

Across the eight paired passing runs, the timing delta was `-16.962s` with
interval `(-28.078, -6.259)`, and `workspace_cli` was faster in six of eight
pairs. The command-count delta was `-6.125` commands with interval
`(-8.375, -3.750)`. By task, the timing deltas were
`invoice_tax_sync = -6.842s`, `policy_threshold_sync = +1.391s`,
`rollback_recovery = -33.107s`, and
`subscription_rollout_sync = -29.291s`. Rollback and subscription were faster in
both pairs, invoice was split one-to-one while still using seven fewer commands
on average, and policy remained a near-parity small task.

The pilot did produce one direct product improvement. A pre-fix run showed that
parallel Codex-issued `workspace read` operations could interleave writes to
`.workspace/log.jsonl`, making `workspace status` report `operation log
unreadable`. `append_operation_log` now takes an exclusive file lock before
writing each JSONL record. The locked-log pilot had 17 valid workspace log
entries and no `operation log unreadable` status.

## What This Proves

- Codex can be run non-interactively against controlled development tasks.
- Codex can be prompted to use `workspace-cli` for real observation,
  verification, patch, replace, rollback, related-file, and impact operations.
- The harness records enough evidence to compare success, overhead, command
  choice, final diffs, and workspace audit logs.
- The timing evidence is now task-dependent rather than uniformly negative:
  simple checkout and the initial co-change prompts were slower with
  `workspace-cli`, the optimized policy task now uses fewer commands and is near
  elapsed-time parity, and the larger invoice task uses far fewer commands with
  a same-commit non-zero-crossing timing improvement. The optimized rollback suite is
  the first repeated single-task Codex-in-the-loop result where `workspace_cli`
  was faster in every paired run and the paired timing interval did not cross
  zero. The latest three-task suite shows a same-commit twelve-pair aggregate
  timing improvement with a non-zero-crossing interval and substantially fewer
  commands, and the merged twelve-pair evidence artifact independently points in
  the same direction. The subscription rollout task exposed a bottleneck in
  manual multi-file patch construction; adding `workspace replace` converted that
  larger task from noisy/parity evidence into a clean four-run speedup with
  perfect workspace diff-scope correctness and a large command-count reduction.
  Adding `workspace trial` then converted rollback recovery from the remaining
  regression into a clean four-run speedup with a non-zero-crossing interval.
  The updated four-task same-commit aggregate also has a non-zero-crossing timing
  improvement after `workspace trial`, with rollback no longer the regressing
  task. The paper can now make controlled-task speedup claims for transactional
  recovery, invoice-style multi-file synchronization, subscription rollout
  synchronization, and same-commit aggregate behavior.

## Next Required Step

The next evaluation should increase repetitions on the four-task aggregate with
`workspace trial` included and refresh the paper artifact bundle from that clean
commit. The report should keep separating single-task speed claims, larger-task
safety claims, merged artifact claims, and same-commit cross-task aggregate
claims.
