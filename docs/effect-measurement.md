# Effect Measurement

This document distinguishes effect measurement from smoke testing.

Smoke tests answer whether commands run. Effect measurement asks whether
`workspace` improves the workspace work loop:

- Can it surface useful workspace facts in one observation?
- Can history-based relation signals find files that text or diff-only views miss?
- Can a change be audited and rolled back with evidence?

## Reproduce

```sh
python3 tools/measure_effect.py
```

The script creates temporary git repositories, runs the real `workspace`
binary, and reports JSON metrics.

To render paper-ready Markdown tables from a JSON report:

```sh
python3 tools/measure_effect.py > /tmp/workspace-effect.json
python3 tools/summarize_effect.py /tmp/workspace-effect.json
```

To add an optional temporal holdout measurement on a real repository, pass a
repository path:

```sh
python3 tools/measure_effect.py --repo-holdout . --max-heldout-commits 5
```

Repeat `--repo-holdout` to evaluate several repositories and emit a cross-repo
aggregate:

```sh
python3 tools/measure_effect.py --repo-holdout . --repo-holdout ../other-repo
```

Pass `--repo-holdout-ref` once per repository to pin the end of each holdout
history. This makes representative measurements reproducible even as the
repositories receive later commits:

```sh
python3 tools/measure_effect.py \
  --repo-holdout . \
  --repo-holdout-ref HEAD
```

Use `--k` to change the primary ranking cutoff. The report includes a
`cutoff_sweep` for the default cutoffs at or below `k`, plus `k` itself.
Use `--hybrid-direct-weight-sweep` to evaluate additional hybrid direct weights
without changing the CLI defaults:

```sh
python3 tools/measure_effect.py --hybrid-direct-weight-sweep 0,0.05,0.5,1
```

This checks out each held-out commit's parent in a temporary clone, builds the
co-change index from only the older history, and measures whether
`workspace related` predicts the files that changed together in the held-out
commit.

## Metrics

### Map Fact Recall

Measures whether `workspace map --json` surfaces known workspace facts from a
fixture:

- package manager
- entrypoint
- test file
- config file
- README
- test command
- next observation

### Related/Impact Recall@3

Measures whether co-change history recovers expected impacted files.

The fixture has this history graph:

```text
src/auth.rs -> src/session.rs -> src/cookie.rs -> tests/cookie_test.rs
```

Expected impacted files for an auth change:

```text
src/session.rs
src/cookie.rs
tests/cookie_test.rs
```

The measurement compares:

- `git diff --name-only`
- `workspace related --use-index`
- `workspace related --rank pagerank`
- `workspace related --rank hybrid`
- `workspace related --rank hybrid --hybrid-direct-weight <0.0-1.0>`
- `workspace impact --diff --use-index`
- `workspace impact --diff --rank pagerank`
- `workspace impact --diff --rank hybrid`
- `workspace impact --diff --rank hybrid --hybrid-direct-weight <0.0-1.0>`

### Retrieval Suite

Measures related-file discovery across several small, reproducible git-history
topologies instead of relying on one perfect fixture:

- transitive chain: code to session to cookie to test
- broad generated update: a large generated commit should be filtered out
- multi-seed bridge: two changed files share a dependency that reaches a test
- hard negatives: direct documentation co-changes compete with an indirect test

For each scenario the script reports the following metrics at the primary
cutoff, which defaults to 5:

- precision@k
- recall@k
- average precision@k
- mean reciprocal rank
- nDCG@k
- deterministic bootstrap 95% confidence intervals for aggregate means
- paired mean deltas, win/tie/loss counts, and paired sign-flip randomization
  p-values
- a default cutoff sweep at @1, @3, and @5
- an optional hybrid direct-weight sweep for ablation
- an optional leave-one-repo-out direct-weight selection check when multiple
  temporal holdout repositories and sweep weights are provided

The suite compares `git diff --name-only`, a seed-agnostic recent-activity
baseline, seed-agnostic global PageRank over the co-change graph, direct
co-change ranking, personalized PageRank over the saved co-change index, hybrid
ranking that combines direct co-change evidence with PageRank reachability, and
the impact-specific PageRank ranking that lightly prioritizes tests over
documentation noise.
The recent-activity baseline ranks tracked files by their latest prior Git
activity while excluding the seed files, so it controls for generally hot files
without using any seed-specific relationship signal.
The global PageRank baseline ranks graph-central files without personalizing to
the seed, so it controls for centrality in the co-change graph.

### Temporal Holdout

Optionally measures prediction against real repository history. For each
eligible held-out commit, the seed is one file from that commit and the expected
set is the other files changed in the same commit. The training history is the
commit's parent and earlier ancestors only. This keeps future co-change edges
out of the index and makes the metric closer to a realistic prospective
prediction task.
When several repositories are provided, the script keeps the per-repository
measurements and also reports a `repo_temporal_holdout_aggregate` metric over
all held-out seed cases.
The temporal holdout report also includes `predictable_only`, which re-scores
the same rankings against expected target files that already existed at the
held-out commit's parent. This separates ordinary related-file prediction from
new-file creation targets that no history-based method can name before they
exist.

### Transaction Audit Signal Recall

Measures whether a patch workflow produces the evidence needed to audit and
reverse a change:

- transaction id
- changed files
- diff after patch
- verification exit code
- operation log entries
- rollback restored the file
- clean diff after rollback

## Current Result

Run `python3 tools/measure_effect.py` to refresh fixture numbers, and add
`--repo-holdout . --repo-holdout-ref 104bbc9155b2ab7df8159f6cb1efe26cd8e95a48`
to reproduce the repo holdout numbers below. A representative result for the
current MVP is:

```text
map_fact_recall: 1.000
git_diff_only recall@3: 0.000
workspace_related_direct recall@3: 0.333
workspace_related_pagerank recall@3: 1.000
workspace_related_hybrid recall@3: 1.000
workspace_impact_direct recall@3: 0.333
workspace_impact_pagerank recall@3: 1.000
workspace_impact_hybrid recall@3: 1.000
retrieval_suite git_diff_only mean_recall@5: 0.000
retrieval_suite recent_activity mean_recall@5: 0.750
retrieval_suite recent_activity mean_average_precision@5: 0.451
retrieval_suite recent_activity mean_ndcg@5: 0.530
retrieval_suite global_pagerank mean_recall@5: 0.917
retrieval_suite global_pagerank mean_average_precision@5: 0.625
retrieval_suite global_pagerank mean_ndcg@5: 0.755
retrieval_suite related_direct mean_recall@5: 0.611
retrieval_suite related_pagerank mean_recall@5: 1.000
retrieval_suite related_pagerank mean_average_precision@5: 0.900
retrieval_suite related_pagerank mean_ndcg@5: 0.950
retrieval_suite related_hybrid mean_recall@5: 1.000
retrieval_suite related_hybrid mean_average_precision@5: 0.900
retrieval_suite related_hybrid mean_ndcg@5: 0.950
retrieval_suite impact_direct mean_recall@5: 0.583
retrieval_suite impact_direct mean_average_precision@5: 0.489
retrieval_suite impact_direct mean_ndcg@5: 0.587
retrieval_suite impact_pagerank mean_recall@5: 1.000
retrieval_suite impact_pagerank mean_average_precision@5: 1.000
retrieval_suite impact_pagerank mean_ndcg@5: 1.000
retrieval_suite impact_hybrid mean_recall@5: 1.000
retrieval_suite impact_hybrid mean_average_precision@5: 1.000
retrieval_suite impact_hybrid mean_ndcg@5: 1.000
repo_holdout recent_activity mean_recall@5: 1.000
repo_holdout recent_activity mean_average_precision@5: 0.799
repo_holdout recent_activity mean_ndcg@5: 0.856
repo_holdout direct mean_recall@5: 0.885
repo_holdout direct mean_average_precision@5: 0.874
repo_holdout direct mean_ndcg@5: 0.913
repo_holdout pagerank mean_recall@5: 1.000
repo_holdout pagerank mean_average_precision@5: 0.641
repo_holdout pagerank mean_ndcg@5: 0.730
repo_holdout hybrid mean_recall@5: 1.000
repo_holdout hybrid mean_average_precision@5: 0.962
repo_holdout hybrid mean_ndcg@5: 0.972
transaction_audit_signal_recall: 1.000
```

Representative paired deltas over the retrieval suite. Parentheses show
deterministic bootstrap 95% confidence intervals for the mean paired delta. The
JSON output also includes win/tie/loss counts and one-sided/two-sided paired
sign-flip p-values for each delta:

```text
retrieval_suite related_hybrid - direct average_precision@5: +0.414 (0.000, 0.667)
retrieval_suite related_hybrid - direct ndcg@5: +0.372 (0.000, 0.586)
retrieval_suite related_hybrid - pagerank average_precision@5: +0.000 (0.000, 0.000)
retrieval_suite related_hybrid - pagerank ndcg@5: +0.000 (0.000, 0.000)
retrieval_suite related_hybrid - recent_activity average_precision@5: +0.632 (0.375, 1.000)
retrieval_suite related_hybrid - recent_activity ndcg@5: +0.577 (0.349, 1.000)
retrieval_suite related_hybrid - global_pagerank average_precision@5: +0.317 (-0.050, 0.667)
retrieval_suite related_hybrid - global_pagerank ndcg@5: +0.236 (-0.027, 0.500)
retrieval_suite impact_hybrid - direct average_precision@5: +0.510 (0.167, 0.781)
retrieval_suite impact_hybrid - direct ndcg@5: +0.413 (0.097, 0.649)
retrieval_suite impact_hybrid - pagerank average_precision@5: +0.000 (0.000, 0.000)
retrieval_suite impact_hybrid - pagerank ndcg@5: +0.000 (0.000, 0.000)
retrieval_suite impact_hybrid - recent_activity average_precision@5: +0.549 (0.131, 0.881)
retrieval_suite impact_hybrid - recent_activity ndcg@5: +0.470 (0.125, 0.846)
retrieval_suite impact_hybrid - global_pagerank average_precision@5: +0.375 (0.250, 0.563)
retrieval_suite impact_hybrid - global_pagerank ndcg@5: +0.245 (0.123, 0.434)
```

A three-repository temporal holdout run can be reproduced with:

```sh
python3 tools/measure_effect.py \
  --repo-holdout . \
  --repo-holdout-ref 104bbc9155b2ab7df8159f6cb1efe26cd8e95a48 \
  --repo-holdout ../related-cli \
  --repo-holdout-ref 5cf1f671993ff93b908dd23e46819a10408042c2 \
  --repo-holdout ../llm-json-extract \
  --repo-holdout-ref 9631a65ab4797fb9260d90fc68db9526811a3be6 \
  --max-heldout-commits 3 \
  --max-candidate-commits 20 \
  --hybrid-direct-weight-sweep 0,0.05,0.5,1
```

Dataset composition for that run:

```text
cross_repo candidates: 60, examined: 11, heldout commits: 9
cross_repo cases: 24, targets: 72, predictable cases: 22, predictable targets: 58, unpredictable targets: 14
cross_repo targets/case mean (min/median/max): 3.000 (1/3.000/5)
cross_repo predictable targets/case mean (min/median/max): 2.636 (1/2.000/4)
cross_repo skipped root=0, too_few_files=2, too_many_files=0, new_seed_files=10
workspace-cli candidates: 20, examined: 4, heldout: 3, cases: 6, targets: 6, predictable targets: 6, skipped too_few_files=1
related-cli candidates: 20, examined: 3, heldout: 3, cases: 7, targets: 23, predictable targets: 12, unpredictable targets: 11, skipped new_seed_files=7
llm-json-extract candidates: 20, examined: 4, heldout: 3, cases: 11, targets: 43, predictable targets: 40, unpredictable targets: 3, skipped too_few_files=1, new_seed_files=3
```

Representative all-target aggregate over 9 held-out commits, 24 seed cases, and
72 target file labels. Parentheses show deterministic bootstrap 95% confidence
intervals for the mean:

```text
cross_repo recent_activity recall@5: 0.646 (0.493, 0.778)
cross_repo recent_activity average_precision@5: 0.455 (0.322, 0.587)
cross_repo recent_activity ndcg@5: 0.541 (0.410, 0.662)
cross_repo global_pagerank recall@5: 0.778 (0.625, 0.903)
cross_repo global_pagerank average_precision@5: 0.473 (0.332, 0.627)
cross_repo global_pagerank ndcg@5: 0.568 (0.432, 0.698)
cross_repo direct recall@5: 0.806 (0.667, 0.931)
cross_repo direct average_precision@5: 0.689 (0.547, 0.826)
cross_repo direct ndcg@5: 0.741 (0.600, 0.864)
cross_repo pagerank recall@5: 0.806 (0.667, 0.931)
cross_repo pagerank average_precision@5: 0.613 (0.477, 0.739)
cross_repo pagerank ndcg@5: 0.692 (0.555, 0.813)
cross_repo hybrid recall@5: 0.806 (0.667, 0.931)
cross_repo hybrid average_precision@5: 0.748 (0.620, 0.869)
cross_repo hybrid ndcg@5: 0.794 (0.669, 0.909)
cross_repo hybrid - direct average_precision@5: +0.059 (0.025, 0.097), wins/ties/losses 10/14/0, p_greater=0.0011
cross_repo hybrid - direct ndcg@5: +0.053 (0.023, 0.082), wins/ties/losses 10/14/0, p_greater=0.0008
cross_repo hybrid - pagerank average_precision@5: +0.135 (0.031, 0.250), wins/ties/losses 5/19/0, p_greater=0.0318
cross_repo hybrid - pagerank ndcg@5: +0.102 (0.031, 0.181), wins/ties/losses 5/19/0, p_greater=0.0311
cross_repo hybrid - recent_activity average_precision@5: +0.293 (0.182, 0.395), wins/ties/losses 17/6/1, p_greater=0.0002
cross_repo hybrid - recent_activity ndcg@5: +0.252 (0.141, 0.358), wins/ties/losses 17/6/1, p_greater=0.0005
cross_repo hybrid - global_pagerank average_precision@5: +0.275 (0.148, 0.401), wins/ties/losses 11/13/0, p_greater=0.0005
cross_repo hybrid - global_pagerank ndcg@5: +0.225 (0.129, 0.334), wins/ties/losses 11/13/0, p_greater=0.0003
cross_repo pagerank - direct average_precision@5: -0.076 (-0.205, 0.035), wins/ties/losses 10/9/5, p_greater=0.8742
cross_repo pagerank - direct ndcg@5: -0.049 (-0.153, 0.039), wins/ties/losses 10/9/5, p_greater=0.8239
```

Per-repository means show where the aggregate gain comes from:

```text
workspace-cli cases: 6, targets: 6, recent AP@5: 0.792, global PageRank AP@5: 0.250, direct AP@5: 1.000, pagerank AP@5: 0.458, hybrid AP@5: 1.000
related-cli cases: 7, targets: 23, recent AP@5: 0.135, global PageRank AP@5: 0.138, direct AP@5: 0.302, pagerank AP@5: 0.437, hybrid AP@5: 0.437
llm-json-extract cases: 11, targets: 43, recent AP@5: 0.475, global PageRank AP@5: 0.809, direct AP@5: 0.765, pagerank AP@5: 0.809, hybrid AP@5: 0.809
```

For predictable-only targets, 22 seed cases and 58 target labels remain:

```text
predictable cross_repo recent_activity average_precision@5: 0.518 (0.398, 0.636)
predictable cross_repo global_pagerank average_precision@5: 0.538 (0.398, 0.687)
predictable cross_repo direct average_precision@5: 0.799 (0.697, 0.899)
predictable cross_repo pagerank average_precision@5: 0.738 (0.595, 0.856)
predictable cross_repo hybrid average_precision@5: 0.885 (0.781, 0.957)
predictable cross_repo hybrid - direct average_precision@5: +0.086 (0.035, 0.142), wins/ties/losses 10/12/0, p_greater=0.0011
predictable cross_repo hybrid - pagerank average_precision@5: +0.148 (0.045, 0.273), wins/ties/losses 5/17/0, p_greater=0.0305
predictable cross_repo hybrid - recent_activity average_precision@5: +0.368 (0.214, 0.512), wins/ties/losses 17/4/1, p_greater=0.0001
predictable cross_repo hybrid - global_pagerank average_precision@5: +0.347 (0.196, 0.491), wins/ties/losses 11/11/0, p_greater=0.0006
```

Predictable-only per-repository means:

```text
workspace-cli predictable cases: 6, targets: 6, recent AP@5: 0.792, global PageRank AP@5: 0.250, direct AP@5: 1.000, pagerank AP@5: 0.458, hybrid AP@5: 1.000
related-cli predictable cases: 6, targets: 12, recent AP@5: 0.236, global PageRank AP@5: 0.242, direct AP@5: 0.528, pagerank AP@5: 0.764, hybrid AP@5: 0.764
llm-json-extract predictable cases: 10, targets: 40, recent AP@5: 0.522, global PageRank AP@5: 0.889, direct AP@5: 0.842, pagerank AP@5: 0.889, hybrid AP@5: 0.889
```

The report also includes `repo_macro_average`, which treats each repository as
one unit instead of weighting by seed-case count:

```text
repo_macro recent_activity average_precision@5: 0.467 (0.135, 0.792)
repo_macro global_pagerank average_precision@5: 0.399 (0.138, 0.809)
repo_macro direct average_precision@5: 0.689 (0.302, 1.000)
repo_macro pagerank average_precision@5: 0.568 (0.437, 0.809)
repo_macro hybrid average_precision@5: 0.749 (0.437, 1.000)
repo_macro hybrid - direct average_precision@5: +0.059 (0.000, 0.135), wins/ties/losses 2/1/0
repo_macro hybrid - pagerank average_precision@5: +0.181 (0.000, 0.542), wins/ties/losses 1/2/0
repo_macro hybrid - recent_activity average_precision@5: +0.281 (0.208, 0.334), wins/ties/losses 3/0/0
repo_macro hybrid - global_pagerank average_precision@5: +0.349 (0.000, 0.750), wins/ties/losses 2/1/0
```

Predictable-only repo macro average:

```text
predictable repo_macro recent_activity average_precision@5: 0.517 (0.236, 0.792)
predictable repo_macro global_pagerank average_precision@5: 0.460 (0.242, 0.889)
predictable repo_macro direct average_precision@5: 0.790 (0.528, 1.000)
predictable repo_macro pagerank average_precision@5: 0.704 (0.458, 0.889)
predictable repo_macro hybrid average_precision@5: 0.884 (0.764, 1.000)
predictable repo_macro hybrid - direct average_precision@5: +0.095 (0.000, 0.236), wins/ties/losses 2/1/0
predictable repo_macro hybrid - recent_activity average_precision@5: +0.368 (0.208, 0.528), wins/ties/losses 3/0/0
predictable repo_macro hybrid - global_pagerank average_precision@5: +0.424 (0.000, 0.750), wins/ties/losses 2/1/0
```

The report also includes a `cutoff_sweep` array for the same held-out cases.
Representative cross-repo average precision by cutoff:

```text
cross_repo direct average_precision@1: 0.340
cross_repo recent_activity average_precision@1: 0.208
cross_repo global_pagerank average_precision@1: 0.094
cross_repo pagerank average_precision@1: 0.205
cross_repo hybrid average_precision@1: 0.413
cross_repo hybrid - direct average_precision@1: +0.073 (0.024, 0.122), wins/ties/losses 6/18/0, p_greater=0.0170
cross_repo hybrid - pagerank average_precision@1: +0.208 (0.042, 0.375), wins/ties/losses 5/19/0, p_greater=0.0314
cross_repo direct average_precision@3: 0.546
cross_repo recent_activity average_precision@3: 0.335
cross_repo global_pagerank average_precision@3: 0.252
cross_repo pagerank average_precision@3: 0.442
cross_repo hybrid average_precision@3: 0.608
cross_repo hybrid - direct average_precision@3: +0.062 (0.024, 0.110), wins/ties/losses 8/16/0, p_greater=0.0027
cross_repo hybrid - pagerank average_precision@3: +0.167 (0.042, 0.312), wins/ties/losses 5/19/0, p_greater=0.0303
```

A representative cross-repo hybrid direct-weight sweep over the same held-out
cases:

```text
cross_repo hybrid direct_weight=0.00 average_precision@5: 0.613
cross_repo hybrid direct_weight=0.05 average_precision@5: 0.644
cross_repo hybrid direct_weight=0.50 average_precision@5: 0.748
cross_repo hybrid direct_weight=1.00 average_precision@5: 0.689
cross_repo hybrid direct_weight=0.50 - direct average_precision@5: +0.059 (0.028, 0.093), p_greater=0.0010
cross_repo hybrid direct_weight=0.50 - pagerank average_precision@5: +0.135 (0.042, 0.240), p_greater=0.0335
```

To check whether the default hybrid direct weight is just overfit to the same
cross-repo evaluation, the summary also reports leave-one-repo-out weight
selection. Each fold chooses the best weight on the other two repositories, then
evaluates that selected weight on the held-out repository:

```text
LORO all-target workspace-cli selected_weight=0.50, train AP@5: 0.664, test AP@5: 1.000
LORO all-target related-cli selected_weight=0.50, train AP@5: 0.876, test AP@5: 0.437
LORO all-target llm-json-extract selected_weight=0.50, train AP@5: 0.697, test AP@5: 0.809
LORO all-target aggregate AP@5: 0.748 (0.615, 0.864)
LORO all-target hybrid - direct average_precision@5: +0.059 (0.029, 0.095), wins/ties/losses 10/14/0, p_greater=0.0016
LORO all-target hybrid - pagerank average_precision@5: +0.135 (0.042, 0.240), wins/ties/losses 5/19/0, p_greater=0.0339
LORO all-target hybrid - recent_activity average_precision@5: +0.293 (0.189, 0.392), wins/ties/losses 17/6/1, p_greater=0.0001
LORO all-target hybrid - global_pagerank average_precision@5: +0.275 (0.150, 0.407), wins/ties/losses 11/13/0, p_greater=0.0006
LORO predictable workspace-cli selected_weight=0.50, train AP@5: 0.842, test AP@5: 1.000
LORO predictable related-cli selected_weight=0.50, train AP@5: 0.931, test AP@5: 0.764
LORO predictable llm-json-extract selected_weight=0.50, train AP@5: 0.882, test AP@5: 0.889
LORO predictable aggregate AP@5: 0.885 (0.779, 0.956)
LORO predictable hybrid - direct average_precision@5: +0.086 (0.040, 0.143), wins/ties/losses 10/12/0, p_greater=0.0013
LORO predictable hybrid - recent_activity average_precision@5: +0.368 (0.218, 0.501), wins/ties/losses 17/4/1, p_greater=0.0001
LORO predictable hybrid - global_pagerank average_precision@5: +0.347 (0.211, 0.497), wins/ties/losses 11/11/0, p_greater=0.0009
```

Interpretation: the CLI is not just running; it measurably improves observation
coverage and related-file discovery across direct, indirect, noisy, and
multi-seed fixtures, while preserving auditable change/rollback evidence.
