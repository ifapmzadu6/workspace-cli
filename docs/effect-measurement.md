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

To generate the JSON report, threshold log, Markdown summary, and run manifest
in one artifact directory:

```sh
python3 tools/run_effect_artifacts.py --paper --output-dir target/effect-paper
```

The JSON report includes reproducibility metadata: the workspace commit, dirty
state, primary cutoff, resampling counts, sign-flip p-value method, holdout
manifest path/hash, and pinned holdout repositories. The Markdown summary
renders the same metadata before the metric tables.
For paper-style holdout reports, the threshold log gates both AP effect-size
floors and Holm-adjusted paired sign-flip p-value ceilings for the key
hybrid-vs-baseline deltas.

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
python3 tools/measure_effect.py --hybrid-direct-weight-sweep 0,0.05,0.1,0.25,0.5,0.6,0.75,0.8,0.9,1
```

The fixed-ref cross-repository setup used for the paper-style results is stored
in `tools/effect_paper_holdouts.json`:

```sh
python3 tools/measure_effect.py \
  --repo-holdout-manifest tools/effect_paper_holdouts.json
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
- paired mean deltas, win/tie/loss counts, exact paired sign-flip p-values
  when the metric grid is finite, and deterministic sampled fallback p-values,
  including Holm-adjusted p-values within each comparison family
- oracle-normalized AP and oracle gap for temporal holdout measurements that
  include the history-only oracle ceiling
- a default cutoff sweep at @1, @3, and @5
- an optional hybrid direct-weight sweep for ablation
- an optional leave-one-repo-out direct-weight selection check when multiple
  temporal holdout repositories and sweep weights are provided
- paper-style threshold gates for both AP deltas and corrected paired
  significance against direct, PageRank, lexical, content, recent-activity, and
  global-PageRank baselines

The suite compares `git diff --name-only`, seed-specific path-locality,
lexical-similarity, and content-similarity baselines, a seed-agnostic
recent-activity baseline, seed-agnostic global PageRank over the co-change
graph, direct co-change ranking, personalized PageRank over the saved co-change
index, hybrid ranking that combines direct co-change evidence with PageRank
reachability, and the impact-specific PageRank ranking that lightly prioritizes
tests over documentation noise.
The path-locality baseline ranks tracked files by shared parent directories and
file extensions with the seed files, so it controls for a cheap static
seed-specific signal without using history.
The lexical-similarity baseline ranks tracked files by token overlap in file
and directory names after dropping common structural tokens such as source,
test, documentation, and file-extension markers. It controls for cheap static
name matching without using history.
The content-similarity baseline ranks tracked files by TF-IDF cosine similarity
between seed file contents and candidate file contents, without using history.
It controls for a cheap static content-retrieval explanation of the result.
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
The all-target holdout also reports a `history_oracle_ceiling` method. It ranks
the predictable target files first, so it is an upper bound for any method that
can only return files already present in the training history.

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
retrieval_suite path_locality mean_recall@5: 1.000
retrieval_suite path_locality mean_average_precision@5: 0.668
retrieval_suite path_locality mean_ndcg@5: 0.782
retrieval_suite lexical_similarity mean_recall@5: 0.708
retrieval_suite lexical_similarity mean_average_precision@5: 0.400
retrieval_suite lexical_similarity mean_ndcg@5: 0.502
retrieval_suite content_similarity mean_recall@5: 0.792
retrieval_suite content_similarity mean_average_precision@5: 0.560
retrieval_suite content_similarity mean_ndcg@5: 0.661
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
JSON output also includes win/tie/loss counts, one-sided/two-sided paired
sign-flip p-values, and Holm-adjusted p-values for each delta family. The
script uses exact dynamic-programming sign-flip counts for the rounded ranking
metric grid and falls back to deterministic sampling only if the state space is
too large:

```text
retrieval_suite related_hybrid - direct average_precision@5: +0.414 (0.000, 0.667)
retrieval_suite related_hybrid - direct ndcg@5: +0.372 (0.000, 0.586)
retrieval_suite related_hybrid - pagerank average_precision@5: +0.000 (0.000, 0.000)
retrieval_suite related_hybrid - pagerank ndcg@5: +0.000 (0.000, 0.000)
retrieval_suite related_hybrid - path_locality average_precision@5: +0.204 (-0.133, 0.500)
retrieval_suite related_hybrid - path_locality ndcg@5: +0.138 (-0.070, 0.369)
retrieval_suite related_hybrid - lexical_similarity average_precision@5: +0.506 (0.000, 0.917)
retrieval_suite related_hybrid - lexical_similarity ndcg@5: +0.470 (0.000, 0.798)
retrieval_suite related_hybrid - content_similarity average_precision@5: +0.404 (0.000, 0.611)
retrieval_suite related_hybrid - content_similarity ndcg@5: +0.361 (0.000, 0.613)
retrieval_suite related_hybrid - recent_activity average_precision@5: +0.632 (0.375, 1.000)
retrieval_suite related_hybrid - recent_activity ndcg@5: +0.577 (0.349, 1.000)
retrieval_suite related_hybrid - global_pagerank average_precision@5: +0.317 (-0.050, 0.667)
retrieval_suite related_hybrid - global_pagerank ndcg@5: +0.236 (-0.027, 0.500)
retrieval_suite impact_hybrid - direct average_precision@5: +0.510 (0.167, 0.781)
retrieval_suite impact_hybrid - direct ndcg@5: +0.413 (0.097, 0.649)
retrieval_suite impact_hybrid - pagerank average_precision@5: +0.000 (0.000, 0.000)
retrieval_suite impact_hybrid - pagerank ndcg@5: +0.000 (0.000, 0.000)
retrieval_suite impact_hybrid - path_locality average_precision@5: +0.332 (0.206, 0.479)
retrieval_suite impact_hybrid - path_locality ndcg@5: +0.218 (0.097, 0.338)
retrieval_suite impact_hybrid - lexical_similarity average_precision@5: +0.600 (0.225, 0.909)
retrieval_suite impact_hybrid - lexical_similarity ndcg@5: +0.498 (0.191, 0.780)
retrieval_suite impact_hybrid - content_similarity average_precision@5: +0.440 (0.125, 0.756)
retrieval_suite impact_hybrid - content_similarity ndcg@5: +0.339 (0.061, 0.616)
retrieval_suite impact_hybrid - recent_activity average_precision@5: +0.549 (0.131, 0.881)
retrieval_suite impact_hybrid - recent_activity ndcg@5: +0.470 (0.125, 0.846)
retrieval_suite impact_hybrid - global_pagerank average_precision@5: +0.375 (0.250, 0.563)
retrieval_suite impact_hybrid - global_pagerank ndcg@5: +0.245 (0.123, 0.434)
```

A compact three-repository temporal holdout run can be reproduced with:

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

The compact numbers below show the report shape on a faster run. The expanded
fixed-ref manifest later in this section is the current tuned paper-style
result. Parentheses show deterministic bootstrap 95% confidence intervals for
the mean:

```text
cross_repo path_locality recall@5: 0.194 (0.111, 0.288)
cross_repo path_locality average_precision@5: 0.099 (0.055, 0.150)
cross_repo path_locality ndcg@5: 0.171 (0.101, 0.242)
cross_repo recent_activity recall@5: 0.646 (0.493, 0.778)
cross_repo recent_activity average_precision@5: 0.455 (0.322, 0.587)
cross_repo recent_activity ndcg@5: 0.541 (0.410, 0.662)
cross_repo global_pagerank recall@5: 0.778 (0.625, 0.903)
cross_repo global_pagerank average_precision@5: 0.473 (0.332, 0.627)
cross_repo global_pagerank ndcg@5: 0.568 (0.432, 0.698)
cross_repo history_oracle_ceiling average_precision@5: 0.833 (0.708, 0.931)
cross_repo direct recall@5: 0.806 (0.667, 0.931)
cross_repo direct average_precision@5: 0.689 (0.547, 0.826)
cross_repo direct ndcg@5: 0.741 (0.600, 0.864)
cross_repo pagerank recall@5: 0.806 (0.667, 0.931)
cross_repo pagerank average_precision@5: 0.613 (0.477, 0.739)
cross_repo pagerank ndcg@5: 0.692 (0.555, 0.813)
cross_repo hybrid recall@5: 0.806 (0.667, 0.931)
cross_repo hybrid average_precision@5: 0.748 (0.620, 0.869)
cross_repo hybrid ndcg@5: 0.794 (0.669, 0.909)
cross_repo hybrid oracle-normalized average_precision@5: 0.898, oracle gap: 0.085
cross_repo hybrid - direct average_precision@5: +0.059 (0.025, 0.097), wins/ties/losses 10/14/0, p_greater=0.0010, holm_p_greater=0.0029
cross_repo hybrid - direct ndcg@5: +0.053 (0.023, 0.082), wins/ties/losses 10/14/0, p_greater=0.0010, holm_p_greater=0.0029
cross_repo hybrid - pagerank average_precision@5: +0.135 (0.031, 0.250), wins/ties/losses 5/19/0, p_greater=0.0312, holm_p_greater=0.0625
cross_repo hybrid - pagerank ndcg@5: +0.102 (0.031, 0.181), wins/ties/losses 5/19/0, p_greater=0.0312, holm_p_greater=0.0625
cross_repo hybrid - path_locality average_precision@5: +0.649 (0.490, 0.790), wins/ties/losses 21/2/1, p_greater=<0.0001, holm_p_greater=<0.0001
cross_repo hybrid - path_locality ndcg@5: +0.623 (0.469, 0.767), wins/ties/losses 21/2/1, p_greater=<0.0001, holm_p_greater=<0.0001
cross_repo hybrid - recent_activity average_precision@5: +0.293 (0.182, 0.395), wins/ties/losses 17/6/1, p_greater=<0.0001, holm_p_greater=0.0002
cross_repo hybrid - recent_activity ndcg@5: +0.252 (0.141, 0.358), wins/ties/losses 17/6/1, p_greater=0.0002, holm_p_greater=0.0008
cross_repo hybrid - global_pagerank average_precision@5: +0.275 (0.148, 0.401), wins/ties/losses 11/13/0, p_greater=0.0005, holm_p_greater=0.0020
cross_repo hybrid - global_pagerank ndcg@5: +0.225 (0.129, 0.334), wins/ties/losses 11/13/0, p_greater=0.0005, holm_p_greater=0.0020
cross_repo pagerank - direct average_precision@5: -0.076 (-0.205, 0.035), wins/ties/losses 10/9/5, p_greater=0.8720, holm_p_greater=0.8720
cross_repo pagerank - direct ndcg@5: -0.049 (-0.153, 0.039), wins/ties/losses 10/9/5, p_greater=0.8262, holm_p_greater=0.8262
```

Per-repository means show where the aggregate gain comes from:

```text
workspace-cli cases: 6, targets: 6, path AP@5: 0.000, recent AP@5: 0.792, global PageRank AP@5: 0.250, direct AP@5: 1.000, pagerank AP@5: 0.458, hybrid AP@5: 1.000
related-cli cases: 7, targets: 23, path AP@5: 0.154, recent AP@5: 0.135, global PageRank AP@5: 0.138, direct AP@5: 0.302, pagerank AP@5: 0.437, hybrid AP@5: 0.437
llm-json-extract cases: 11, targets: 43, path AP@5: 0.117, recent AP@5: 0.475, global PageRank AP@5: 0.809, direct AP@5: 0.765, pagerank AP@5: 0.809, hybrid AP@5: 0.809
```

For predictable-only targets, 22 seed cases and 58 target labels remain:

```text
predictable cross_repo path_locality average_precision@5: 0.132 (0.072, 0.203)
predictable cross_repo recent_activity average_precision@5: 0.518 (0.398, 0.636)
predictable cross_repo global_pagerank average_precision@5: 0.538 (0.398, 0.687)
predictable cross_repo history_oracle_ceiling average_precision@5: 1.000 (1.000, 1.000)
predictable cross_repo direct average_precision@5: 0.799 (0.697, 0.899)
predictable cross_repo pagerank average_precision@5: 0.738 (0.595, 0.856)
predictable cross_repo hybrid average_precision@5: 0.885 (0.781, 0.957)
predictable cross_repo hybrid oracle-normalized average_precision@5: 0.885, oracle gap: 0.115
predictable cross_repo hybrid - direct average_precision@5: +0.086 (0.035, 0.142), wins/ties/losses 10/12/0, p_greater=0.0010, holm_p_greater=0.0029
predictable cross_repo hybrid - pagerank average_precision@5: +0.148 (0.045, 0.273), wins/ties/losses 5/17/0, p_greater=0.0312, holm_p_greater=0.0625
predictable cross_repo hybrid - path_locality average_precision@5: +0.753 (0.616, 0.876), wins/ties/losses 21/0/1, p_greater=<0.0001, holm_p_greater=<0.0001
predictable cross_repo hybrid - recent_activity average_precision@5: +0.368 (0.214, 0.512), wins/ties/losses 17/4/1, p_greater=<0.0001, holm_p_greater=0.0003
predictable cross_repo hybrid - global_pagerank average_precision@5: +0.347 (0.196, 0.491), wins/ties/losses 11/11/0, p_greater=0.0005, holm_p_greater=0.0020
```

Predictable-only per-repository means:

```text
workspace-cli predictable cases: 6, targets: 6, path AP@5: 0.000, recent AP@5: 0.792, global PageRank AP@5: 0.250, direct AP@5: 1.000, pagerank AP@5: 0.458, hybrid AP@5: 1.000
related-cli predictable cases: 6, targets: 12, path AP@5: 0.270, recent AP@5: 0.236, global PageRank AP@5: 0.242, direct AP@5: 0.528, pagerank AP@5: 0.764, hybrid AP@5: 0.764
llm-json-extract predictable cases: 10, targets: 40, path AP@5: 0.129, recent AP@5: 0.522, global PageRank AP@5: 0.889, direct AP@5: 0.842, pagerank AP@5: 0.889, hybrid AP@5: 0.889
```

The report also includes `repo_macro_average`, which treats each repository as
one unit instead of weighting by seed-case count:

```text
repo_macro path_locality average_precision@5: 0.090 (0.000, 0.154)
repo_macro recent_activity average_precision@5: 0.467 (0.135, 0.792)
repo_macro global_pagerank average_precision@5: 0.399 (0.138, 0.809)
repo_macro direct average_precision@5: 0.689 (0.302, 1.000)
repo_macro pagerank average_precision@5: 0.568 (0.437, 0.809)
repo_macro hybrid average_precision@5: 0.749 (0.437, 1.000)
repo_macro hybrid - direct average_precision@5: +0.059 (0.000, 0.135), wins/ties/losses 2/1/0
repo_macro hybrid - pagerank average_precision@5: +0.181 (0.000, 0.542), wins/ties/losses 1/2/0
repo_macro hybrid - path_locality average_precision@5: +0.658 (0.283, 1.000), wins/ties/losses 3/0/0
repo_macro hybrid - recent_activity average_precision@5: +0.281 (0.208, 0.334), wins/ties/losses 3/0/0
repo_macro hybrid - global_pagerank average_precision@5: +0.349 (0.000, 0.750), wins/ties/losses 2/1/0
```

Predictable-only repo macro average:

```text
predictable repo_macro path_locality average_precision@5: 0.133 (0.000, 0.270)
predictable repo_macro recent_activity average_precision@5: 0.517 (0.236, 0.792)
predictable repo_macro global_pagerank average_precision@5: 0.460 (0.242, 0.889)
predictable repo_macro direct average_precision@5: 0.790 (0.528, 1.000)
predictable repo_macro pagerank average_precision@5: 0.704 (0.458, 0.889)
predictable repo_macro hybrid average_precision@5: 0.884 (0.764, 1.000)
predictable repo_macro hybrid - direct average_precision@5: +0.095 (0.000, 0.236), wins/ties/losses 2/1/0
predictable repo_macro hybrid - path_locality average_precision@5: +0.751 (0.494, 1.000), wins/ties/losses 3/0/0
predictable repo_macro hybrid - recent_activity average_precision@5: +0.368 (0.208, 0.528), wins/ties/losses 3/0/0
predictable repo_macro hybrid - global_pagerank average_precision@5: +0.424 (0.000, 0.750), wins/ties/losses 2/1/0
```

The report also includes a `cutoff_sweep` array for the same held-out cases.
Representative cross-repo average precision by cutoff:

```text
cross_repo direct average_precision@1: 0.340
cross_repo path_locality average_precision@1: 0.031
cross_repo recent_activity average_precision@1: 0.208
cross_repo global_pagerank average_precision@1: 0.094
cross_repo history_oracle_ceiling average_precision@1: 0.437
cross_repo pagerank average_precision@1: 0.205
cross_repo hybrid average_precision@1: 0.413
cross_repo hybrid - direct average_precision@1: +0.073 (0.024, 0.122), wins/ties/losses 6/18/0, p_greater=0.0156, holm_p_greater=0.0469
cross_repo hybrid - pagerank average_precision@1: +0.208 (0.042, 0.375), wins/ties/losses 5/19/0, p_greater=0.0312, holm_p_greater=0.0625
cross_repo direct average_precision@3: 0.546
cross_repo path_locality average_precision@3: 0.072
cross_repo recent_activity average_precision@3: 0.335
cross_repo global_pagerank average_precision@3: 0.252
cross_repo history_oracle_ceiling average_precision@3: 0.729
cross_repo pagerank average_precision@3: 0.442
cross_repo hybrid average_precision@3: 0.608
cross_repo hybrid - direct average_precision@3: +0.062 (0.024, 0.110), wins/ties/losses 8/16/0, p_greater=0.0039, holm_p_greater=0.0117
cross_repo hybrid - pagerank average_precision@3: +0.167 (0.042, 0.312), wins/ties/losses 5/19/0, p_greater=0.0312, holm_p_greater=0.0625
```

The rendered summary also reports case-level AP deltas for the largest wins and
losses, which makes aggregate gains auditable at the seed-file level:

```text
case_delta all-target hybrid - direct win related-cli seed=package.json commit=5cf1f67199 targets=Cargo.lock,Cargo.toml,+1 delta_ap@5=+0.278
case_delta all-target hybrid - path_locality win llm-json-extract seed=CHANGELOG.md commit=0387cf3084 targets=package-lock.json,package.json,+2 delta_ap@5=+1.000
case_delta all-target hybrid - path_locality loss related-cli seed=src/main.rs commit=97835ef97e targets=src/filters.rs,src/model.rs,+1 delta_ap@5=-0.333
case_delta all-target hybrid - recent_activity loss related-cli seed=src/main.rs commit=97835ef97e targets=src/filters.rs,src/model.rs,+1 delta_ap@5=-0.333
case_delta predictable hybrid - recent_activity loss related-cli seed=src/main.rs commit=97835ef97e targets=src/filters.rs,src/output.rs delta_ap@5=-0.500
case_delta predictable hybrid - global_pagerank win workspace-cli seed=docs/effect-measurement.md commit=104bbc9155 targets=tools/measure_effect.py delta_ap@5=+0.750
```

A compact cross-repo hybrid direct-weight sweep over the same held-out cases:

```text
cross_repo hybrid direct_weight=0.00 average_precision@5: 0.613
cross_repo hybrid direct_weight=0.05 average_precision@5: 0.644
cross_repo hybrid direct_weight=0.50 average_precision@5: 0.748
cross_repo hybrid direct_weight=1.00 average_precision@5: 0.689
cross_repo hybrid direct_weight=0.50 - direct average_precision@5: +0.059 (0.028, 0.093), p_greater=0.0010, holm_p_greater=0.0020
cross_repo hybrid direct_weight=0.50 - pagerank average_precision@5: +0.135 (0.042, 0.240), p_greater=0.0312, holm_p_greater=0.0312
```

For the compact coarse grid, the summary also reports leave-one-repo-out weight
selection. Each fold chooses the best weight on the other two repositories,
then evaluates that selected weight on the held-out repository:

```text
LORO all-target workspace-cli selected_weight=0.50, train AP@5: 0.664, test AP@5: 1.000
LORO all-target related-cli selected_weight=0.50, train AP@5: 0.876, test AP@5: 0.437
LORO all-target llm-json-extract selected_weight=0.50, train AP@5: 0.697, test AP@5: 0.809
LORO all-target aggregate AP@5: 0.748 (0.615, 0.864)
LORO all-target hybrid - direct average_precision@5: +0.059 (0.029, 0.095), wins/ties/losses 10/14/0, p_greater=0.0010, holm_p_greater=0.0029
LORO all-target hybrid - pagerank average_precision@5: +0.135 (0.042, 0.240), wins/ties/losses 5/19/0, p_greater=0.0312, holm_p_greater=0.0625
LORO all-target hybrid - path_locality average_precision@5: +0.649 (0.485, 0.780), wins/ties/losses 21/2/1, p_greater=<0.0001, holm_p_greater=<0.0001
LORO all-target hybrid - recent_activity average_precision@5: +0.293 (0.189, 0.392), wins/ties/losses 17/6/1, p_greater=<0.0001, holm_p_greater=0.0002
LORO all-target hybrid - global_pagerank average_precision@5: +0.275 (0.150, 0.407), wins/ties/losses 11/13/0, p_greater=0.0005, holm_p_greater=0.0020
LORO predictable workspace-cli selected_weight=0.50, train AP@5: 0.842, test AP@5: 1.000
LORO predictable related-cli selected_weight=0.50, train AP@5: 0.931, test AP@5: 0.764
LORO predictable llm-json-extract selected_weight=0.50, train AP@5: 0.882, test AP@5: 0.889
LORO predictable aggregate AP@5: 0.885 (0.779, 0.956)
LORO predictable hybrid - direct average_precision@5: +0.086 (0.040, 0.143), wins/ties/losses 10/12/0, p_greater=0.0010, holm_p_greater=0.0029
LORO predictable hybrid - pagerank average_precision@5: +0.148 (0.045, 0.273), wins/ties/losses 5/17/0, p_greater=0.0312, holm_p_greater=0.0625
LORO predictable hybrid - path_locality average_precision@5: +0.753 (0.598, 0.876), wins/ties/losses 21/0/1, p_greater=<0.0001, holm_p_greater=<0.0001
LORO predictable hybrid - recent_activity average_precision@5: +0.368 (0.218, 0.501), wins/ties/losses 17/4/1, p_greater=<0.0001, holm_p_greater=0.0003
LORO predictable hybrid - global_pagerank average_precision@5: +0.347 (0.211, 0.497), wins/ties/losses 11/11/0, p_greater=0.0005, holm_p_greater=0.0020
```

A larger fixed-ref stress run increases the temporal holdout window to 50
candidate commits and up to 5 held-out commits per repository:

```sh
python3 tools/measure_effect.py \
  --repo-holdout-manifest tools/effect_paper_holdouts.json
```

That expanded run contains 15 held-out commits, 50 seed cases, and 207 target
file labels. It preserves the main result with tighter aggregate evidence:

```text
expanded cross_repo hybrid average_precision@5: 0.651 (0.555, 0.741)
expanded cross_repo direct average_precision@5: 0.564 (0.468, 0.655)
expanded cross_repo pagerank average_precision@5: 0.536 (0.443, 0.626)
expanded cross_repo recent_activity average_precision@5: 0.450 (0.357, 0.543)
expanded cross_repo global_pagerank average_precision@5: 0.471 (0.385, 0.562)
expanded cross_repo path_locality average_precision@5: 0.100 (0.069, 0.135)
expanded cross_repo lexical_similarity average_precision@5: 0.240 (0.151, 0.343)
expanded cross_repo content_similarity average_precision@5: 0.378 (0.318, 0.435)
expanded cross_repo history_oracle_ceiling average_precision@5: 0.811 (0.733, 0.878)
expanded cross_repo hybrid oracle-normalized average_precision@5: 0.803, oracle gap: 0.160
expanded cross_repo hybrid - direct average_precision@5: +0.087 (0.049, 0.131), wins/ties/losses 21/24/5, p_greater=<0.0001, holm_p_greater=<0.0001
expanded cross_repo hybrid - pagerank average_precision@5: +0.115 (0.050, 0.186), wins/ties/losses 13/33/4, p_greater=0.0003, holm_p_greater=0.0005
expanded cross_repo hybrid - lexical_similarity average_precision@5: +0.411 (0.314, 0.503), wins/ties/losses 39/11/0, p_greater=<0.0001, holm_p_greater=<0.0001
expanded cross_repo hybrid - content_similarity average_precision@5: +0.273 (0.188, 0.357), wins/ties/losses 35/2/13, p_greater=<0.0001, holm_p_greater=<0.0001
expanded cross_repo hybrid - recent_activity average_precision@5: +0.201 (0.135, 0.271), wins/ties/losses 34/13/3, p_greater=<0.0001, holm_p_greater=<0.0001
expanded cross_repo hybrid - global_pagerank average_precision@5: +0.179 (0.103, 0.263), wins/ties/losses 20/23/7, p_greater=<0.0001, holm_p_greater=<0.0001
expanded predictable cross_repo hybrid average_precision@5: 0.731 (0.638, 0.826)
expanded predictable cross_repo lexical_similarity average_precision@5: 0.264 (0.164, 0.376)
expanded predictable cross_repo content_similarity average_precision@5: 0.419 (0.359, 0.481)
expanded predictable cross_repo history_oracle_ceiling average_precision@5: 0.908 (0.852, 0.963)
expanded predictable cross_repo hybrid oracle-normalized average_precision@5: 0.805, oracle gap: 0.177
expanded predictable cross_repo hybrid - direct average_precision@5: +0.106 (0.062, 0.153), wins/ties/losses 21/22/5, p_greater=<0.0001, holm_p_greater=<0.0001
expanded predictable cross_repo hybrid - pagerank average_precision@5: +0.128 (0.064, 0.199), wins/ties/losses 13/31/4, p_greater=0.0003, holm_p_greater=0.0005
expanded predictable cross_repo hybrid - lexical_similarity average_precision@5: +0.467 (0.355, 0.565), wins/ties/losses 39/9/0, p_greater=<0.0001, holm_p_greater=<0.0001
expanded predictable cross_repo hybrid - content_similarity average_precision@5: +0.312 (0.226, 0.400), wins/ties/losses 35/0/13, p_greater=<0.0001, holm_p_greater=<0.0001
```

The expanded manifest also runs a denser hybrid direct-weight sweep:

```text
expanded cross_repo hybrid direct_weight=0.00 average_precision@5: 0.536
expanded cross_repo hybrid direct_weight=0.05 average_precision@5: 0.558
expanded cross_repo hybrid direct_weight=0.10 average_precision@5: 0.570
expanded cross_repo hybrid direct_weight=0.25 average_precision@5: 0.613
expanded cross_repo hybrid direct_weight=0.50 average_precision@5: 0.640
expanded cross_repo hybrid direct_weight=0.60 average_precision@5: 0.645
expanded cross_repo hybrid direct_weight=0.75 average_precision@5: 0.641
expanded cross_repo hybrid direct_weight=0.80 average_precision@5: 0.651
expanded cross_repo hybrid direct_weight=0.90 average_precision@5: 0.651
expanded cross_repo hybrid direct_weight=1.00 average_precision@5: 0.564
expanded cross_repo hybrid direct_weight=0.80 - direct average_precision@5: +0.087 (0.048, 0.130), p_greater=<0.0001, holm_p_greater=<0.0001
expanded cross_repo hybrid direct_weight=0.80 - pagerank average_precision@5: +0.115 (0.054, 0.179), p_greater=0.0003, holm_p_greater=0.0003
expanded predictable cross_repo hybrid direct_weight=0.50 average_precision@5: 0.719
expanded predictable cross_repo hybrid direct_weight=0.80 average_precision@5: 0.731
```

Leave-one-repo-out selection over that dense grid chooses 0.60 for
`workspace-cli` and 0.80 for the other two repositories. Its aggregate AP@5 is
0.644 versus 0.651 for the fixed 0.80 default, still above the former 0.50
default's 0.640. On the predictable-only slice it selects 0.80 for
`related-cli` and `llm-json-extract`, and its aggregate AP@5 is 0.724 versus
0.731 for the fixed 0.80 default. Because 0.80 improves all-target and
predictable AP while preserving the fixture retrieval result, the CLI related
hybrid default is 0.80.

Interpretation: the CLI is not just running; it measurably improves observation
coverage and related-file discovery across direct, indirect, noisy, and
multi-seed fixtures, while preserving auditable change/rollback evidence.
