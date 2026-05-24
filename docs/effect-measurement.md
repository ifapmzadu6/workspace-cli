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
- `workspace impact --diff --rank pagerank`

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

Run `python3 tools/measure_effect.py` to refresh these numbers. A representative
result for the current MVP is:

```text
map_fact_recall: 1.000
git_diff_only recall@3: 0.000
workspace_related_direct recall@3: 0.333
workspace_related_pagerank recall@3: 1.000
workspace_impact_pagerank recall@3: 1.000
transaction_audit_signal_recall: 1.000
```

Interpretation: the CLI is not just running; it measurably improves observation
coverage and related-file discovery in the fixture, while preserving auditable
change/rollback evidence.
