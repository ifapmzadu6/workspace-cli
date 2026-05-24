# workspace cli

`workspace cli` は、LLMエージェントと人間が同じ操作面を通じてプロジェクト
workspaceを読み、変更し、検証し、状態を追跡するためのCLIである。

狙いは「AI coding assistant」を直接作ることではない。先に作るべきものは、
エージェントが安全かつ効率よくworkspaceを扱うためのruntimeである。

```text
workspace_before + intent
        |
        v
workspace operations
        |
        v
workspace_after + evidence
```

## 現在のMVP

このリポジトリにはRust製のCLI実装が入っている。バイナリ名は `workspace`。

### インストール

開発中は `cargo run -- <command>` で実行できる。ローカルに `workspace` として入れるなら
以下を使う。

```sh
cargo install --path .
workspace --version
workspace --help
```

### 基本コマンド

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

現在のコマンドは以下を提供する。

```text
workspace map       workspaceの地図を作る
workspace status    git状態、index状態、最近の操作を見る
workspace search    ripgrepベースで検索する
workspace index     workspace用indexを作る
workspace related   git履歴の同時変更から関連ファイルを見る
workspace impact    現在の差分から影響候補ファイルを見る
workspace read      テキストファイルまたは行範囲を読む
workspace diff      git diffを見る
workspace patch     patchをtransactionとして適用する
workspace run       コマンドを実行して結果を記録する
workspace log       操作ログを見る
workspace rollback  workspace patchのtransactionを戻す
```

各観測コマンドは `--json` を持ち、LLM agentが扱いやすいように
`summary`、`data`、`evidence`、`next_observations` を含む構造化JSONを返す。
操作ログは `.workspace/log.jsonl` に保存される。

### MVPの検証

現在の品質ゲートは以下。

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
```

単体テストに加えて、実際の `workspace` バイナリを一時workspaceで起動する統合テストを
持っている。`map/read`、co-change index、`related/impact`、`patch/run/log/diff/rollback`
の主要フローを検証する。

効果測定は [docs/effect-measurement.md](/Users/keisukekarijuku/git/workspace-cli/docs/effect-measurement.md)
と `tools/measure_effect.py` にまとめている。ここでは「コマンドが動くか」ではなく、
観測coverage、関連ファイル発見、audit/rollback evidenceがどれだけ得られるかを測る。

### 履歴ベースの関連観測

`workspace related <file> --by cochange` は、ファイル内容ではなくgit履歴を使って
関連ファイルを推定する。同じcommitで一緒に変更されたファイルを関連ありとみなし、
最近のcommitほど強く、変更ファイル数が多いcommitほど弱く重み付けする。

これは「このファイルを触るなら、過去の作業ではどのファイルも一緒に触られたか」
を観測するための機能である。

```sh
workspace related src/config.rs --by cochange --json
workspace related src/config.rs --max-commits 500 --max-files-per-commit 30
```

`related` バイナリが見つかる場合、indexを使わないdirect co-change観測は
[related-cli](https://github.com/ifapmzadu6/related-cli) を優先して使う。
明示的に指定する場合は以下のようにする。

```sh
WORKSPACE_RELATED_BIN=/path/to/related workspace related src/config.rs --by cochange --json
WORKSPACE_RELATED_BIN=/path/to/related workspace impact --diff --by cochange --json
```

`WORKSPACE_RELATED_DISABLE=1` を付けると従来の内部実装だけを使う。
`WORKSPACE_RELATED_HISTORY_BACKEND` で `related-cli` の `--history-backend` を変更でき、
未指定時は正確性を優先して `git` を使う。

大規模リフォーマットやlockfile更新のような広すぎるcommitはノイズになりやすいので、
`--max-files-per-commit` を超えるcommitは関連スコアから除外する。

履歴が大きいworkspaceでは、先にco-change graphを `.workspace/index/cochange.json`
として保存できる。

```sh
workspace index status
workspace index cochange
workspace index cochange --max-commits 2000 --max-files-per-commit 30 --json
workspace related src/config.rs --by cochange --use-index
workspace related src/config.rs --by cochange --rank pagerank
```

`workspace index status` は、保存済みindexが存在するか、現在のgit HEADに対して
freshかstaleかを返す。

`--use-index` と `--rank pagerank` は保存済みco-change graphを使い、seedファイルから関連グラフを
伝播して候補を返す。直接一緒に変更されたファイルだけでなく、その先につながる
ファイルも観測対象にできる。

`workspace impact --diff --by cochange` は、現在のgit差分に含まれるファイルをseedにして、
履歴上よく一緒に変更されてきた周辺ファイルを返す。これは、変更後に追加で読むべき
ファイルや、検証対象になりそうなテスト・ドキュメントを見つけるための観測である。

```sh
workspace impact --diff --by cochange --json
workspace impact --diff --max-commits 500 --max-results 30
workspace impact --diff --by cochange --use-index
workspace impact --diff --by cochange --rank pagerank
```

## 背景

LLM単体の基本的な入出力は、promptを読んでtextを返すことに近い。

```text
input:  prompt
output: text
```

しかし、Codexのような開発エージェントの強さは、会話だけではなくworkspaceを
持っていることにある。ファイルを読める。検索できる。編集できる。コマンドを
実行できる。テスト結果を観測できる。差分を見て、失敗したらやり直せる。

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

この差は大きい。LLMが同じでも、workspace操作がうまいほど実用性能は上がる。
探索、編集、検証、rollbackが速く正確なら、モデルの推論力をより現実の作業に
変換できる。

## コンセプト

`workspace cli` は、workspaceをただのファイルツリーではなく、状態を持つ作業
環境として扱う。

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

エージェントは生のshellやファイルシステムを直接乱暴に触るのではなく、
workspace向けの明示的な操作を使う。

```text
read
search
diff
patch
run
log
rollback
```

これにより、人間にもエージェントにも扱いやすい共通のインターフェースを作る。

## 基本方針

- AIエージェント専用ではなく、人間がCLIとして使っても自然な設計にする。
- ファイル編集はできるだけpatch/transaction中心にする。
- すべての変更にdiff、操作ログ、検証結果を紐づける。
- workspaceの「観測」と「変更」を分ける。
- 最初はlocal git repositoryだけを対象にする。
- 賢いagentより先に、賢いworkspace操作面を作る。

## 最小コマンド案

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

現在のworkspace状態を要約する。

```text
branch
dirty files
untracked files
recent operations
running commands
```

### `workspace search`

workspace内を検索する。初期実装ではripgrep相当でよい。

将来的には、以下を組み合わせる。

```text
text search
symbol search
semantic search
dependency search
doc section search
```

### `workspace read`

ファイルまたは範囲を読む。

```sh
workspace read src/main.rs
workspace read src/main.rs --lines 40:120
```

将来的には、symbol単位やsection単位の読み取りを提供する。

```sh
workspace read-symbol ConfigToml
workspace read-section README.md "Installation"
```

### `workspace diff`

現在の差分を表示する。将来的には、ユーザー由来の変更とエージェント由来の変更を
区別できるようにする。

```text
user changes
agent changes
generated files
test artifacts
```

### `workspace patch`

patchをtransactionとして適用する。

```text
validate
apply
record
show diff
allow rollback
```

直接的な `write_file` よりも、まずはpatch中心にする。レビュー、追跡、rollbackが
しやすいため。

### `workspace run`

コマンドを実行して、出力、終了コード、実行時間を記録する。

```text
command
cwd
env summary
exit code
stdout
stderr
duration
```

将来的には、変更箇所から関連テストを推定して実行する。

### `workspace log`

workspaceに対して行った操作履歴を見る。

```text
read operations
patch operations
commands
test results
rollback points
```

これはエージェントの記憶ではなく、workspace側に残る監査可能な履歴である。

### `workspace rollback`

特定の変更を戻す。git resetのような大きい操作ではなく、workspace cliが適用した
transaction単位で戻すことを目指す。

## 抽象モデル

中心になる型や概念はこのあたり。

```text
WorkspaceState
WorkspaceOp
WorkspaceObservation
WorkspaceTransaction
WorkspaceLog
```

### WorkspaceState

ある時点のworkspace状態。

```text
root path
git branch
git status
known files
index metadata
operation history
```

### WorkspaceOp

workspaceに対する操作。

```text
ReadFile
Search
ApplyPatch
RunCommand
GetDiff
CreateRollbackPoint
Rollback
```

### WorkspaceObservation

操作の結果として得られる観測。

```text
file content
search results
command output
test failure
diff summary
```

### WorkspaceTransaction

ひとまとまりの変更。patch適用、生成ファイル、関連コマンド、検証結果をまとめる。

```text
id
description
operations
files_changed
diff
verification
created_at
```

## エージェントとの関係

LLMエージェントは、workspace cliをtoolとして使う。

```text
User intent
  |
  v
Agent
  |
  v
workspace cli
  |
  v
Workspace
```

このとき、LLMに全ファイルを詰め込むのではなく、必要な観測だけを取得させる。

```text
search -> read -> patch -> run -> diff -> report
```

つまり、コンテキストを増やすのではなく、workspaceへのアクセス効率を上げる。

## MVP

最初のMVPは、以下に絞る。

```text
local git repository only
text files only
read/search/diff
patch apply
command run
operation log
basic rollback
```

高度なsymbol indexやsemantic searchは後回しにする。

MVPで重要なのは、AIが使う以前に、人間が触っても便利なこと。

```sh
workspace status
workspace search "TODO"
workspace read README.md
workspace patch /tmp/change.patch
workspace run "npm test"
workspace diff
workspace log
```

## 将来の拡張

### Symbol index

Tree-sitterやlanguage serverを使って、関数、型、クラス、参照関係を読めるように
する。

```sh
workspace symbols
workspace read-symbol UserSession
workspace references UserSession
```

### Document operations

Markdownやdocsをsection単位で扱う。

```sh
workspace doc sections README.md
workspace doc read-section README.md "Usage"
workspace doc replace-section README.md "Usage" usage.md
```

### Relevant test selection

変更されたファイルやsymbolから、走らせるべきテストを推定する。

```sh
workspace test relevant
workspace test run-relevant
```

### MCP server

CLIだけでなく、MCP serverとしても提供する。

```text
workspace.search
workspace.read
workspace.apply_patch
workspace.run
workspace.diff
workspace.rollback
```

これにより、Codex、Claude、Cursor、その他のagentから同じworkspace操作面を使える。

### Transaction UI

差分、検証結果、rollback pointを見やすくするTUIまたはweb UIを追加する。

## 非目標

- 最初からフル機能のAI coding agentを作らない。
- 最初からIDEを作らない。
- 最初からクラウド同期を作らない。
- 最初から全言語対応の高度なコード理解を作らない。
- 生のshellを完全に置き換えようとしない。

## 重要な問い

- workspace操作の最小プリミティブは何か。
- patch transactionの単位をどう切るべきか。
- 人間が行った変更とagentが行った変更をどう区別するか。
- operation logはgitとどう住み分けるか。
- command実行の安全性と自由度をどう設計するか。
- workspace indexはいつ更新するべきか。
- CLI、SDK、MCP serverの責務をどう分けるか。

## 直近の実装メモ

最初はRustかGoが向いていそうだが、MVPならTypeScriptでもよい。

優先順位は、実装言語よりもデータモデルである。

```text
1. workspace root detection
2. git status/diff integration
3. read/search command
4. patch apply with transaction log
5. run command with captured output
6. rollback support
7. JSON output mode for agents
```

CLIは人間向けの表示とagent向けのJSON出力を分ける。

```sh
workspace status
workspace status --json
```

## 一文で言うと

`workspace cli` は、LLMエージェントと人間のための、観測可能で戻せるworkspace操作面である。
