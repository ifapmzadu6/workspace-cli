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
- `workspace impact --diff --use-index`
- `workspace impact --diff --rank pagerank`
- `workspace impact --diff --rank hybrid`

### Retrieval Suite

Measures related-file discovery across several small, reproducible git-history
topologies instead of relying on one perfect fixture:

- transitive chain: code to session to cookie to test
- broad generated update: a large generated commit should be filtered out
- multi-seed bridge: two changed files share a dependency that reaches a test
- hard negatives: direct documentation co-changes compete with an indirect test

For each scenario the script reports:

- precision@5
- recall@5
- average precision@5
- mean reciprocal rank
- nDCG@5
- deterministic bootstrap 95% confidence intervals for aggregate means
- paired mean deltas, win/tie/loss counts, and paired sign-flip randomization
  p-values

The suite compares `git diff --name-only`, direct co-change ranking,
personalized PageRank over the saved co-change index, hybrid ranking that
combines direct co-change evidence with PageRank reachability, and the
impact-specific PageRank ranking that lightly prioritizes tests over
documentation noise.

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
`--repo-holdout .` to refresh the repo holdout numbers. A representative result
for the current MVP is:

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
repo_holdout direct mean_recall@5: 0.821
repo_holdout direct mean_average_precision@5: 0.752
repo_holdout direct mean_ndcg@5: 0.817
repo_holdout pagerank mean_recall@5: 1.000
repo_holdout pagerank mean_average_precision@5: 0.631
repo_holdout pagerank mean_ndcg@5: 0.726
repo_holdout hybrid mean_recall@5: 1.000
repo_holdout hybrid mean_average_precision@5: 0.887
repo_holdout hybrid mean_ndcg@5: 0.921
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
retrieval_suite impact_hybrid - direct average_precision@5: +0.510 (0.167, 0.781)
retrieval_suite impact_hybrid - direct ndcg@5: +0.413 (0.097, 0.649)
retrieval_suite impact_hybrid - pagerank average_precision@5: +0.000 (0.000, 0.000)
retrieval_suite impact_hybrid - pagerank ndcg@5: +0.000 (0.000, 0.000)
```

A three-repository temporal holdout run can be reproduced with:

```sh
python3 tools/measure_effect.py \
  --repo-holdout . \
  --repo-holdout ../related-cli \
  --repo-holdout ../llm-json-extract \
  --max-heldout-commits 3 \
  --max-candidate-commits 20
```

Representative aggregate over 9 held-out commits and 24 seed cases. Parentheses
show deterministic bootstrap 95% confidence intervals for the mean:

```text
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
cross_repo pagerank - direct average_precision@5: -0.076 (-0.205, 0.035), wins/ties/losses 10/9/5, p_greater=0.8742
cross_repo pagerank - direct ndcg@5: -0.049 (-0.153, 0.039), wins/ties/losses 10/9/5, p_greater=0.8239
```

Interpretation: the CLI is not just running; it measurably improves observation
coverage and related-file discovery across direct, indirect, noisy, and
multi-seed fixtures, while preserving auditable change/rollback evidence.
