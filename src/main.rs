use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
mod related_cli;

use related_cli::{RelatedCli, RelatedCliEvidence, RelatedCliItem, RelatedCliOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const LOG_DIR: &str = ".workspace";
const LOG_FILE: &str = ".workspace/log.jsonl";
const TRANSACTION_DIR: &str = ".workspace/transactions";
const INDEX_DIR: &str = ".workspace/index";
const COCHANGE_INDEX_FILE: &str = ".workspace/index/cochange.json";
const MAX_CAPTURED_OUTPUT: usize = 24_000;
const MAX_READ_CONTENT: usize = 24_000;
const MAX_LOG_SCOPE: usize = 2_000;
const MAX_LOG_SUMMARY: usize = 2_000;
const MAX_CHANGED_FILES: usize = 80;
const MAX_GIT_STATUS_FILES: usize = 80;
const MAX_MAP_LIST_ITEMS: usize = 80;
const MAX_MAP_LARGE_FILES: usize = 40;
const MAX_RECENT_FILES: usize = 12;
const MAX_DIFF_SUMMARY: usize = 12_000;
const MAX_DIFF_PATCH: usize = 48_000;
const MAX_SEARCH_MATCH_TEXT: usize = 2_000;
const MAX_RG_JSON_LINE_BYTES: usize = 64_000;
const MAX_PATCH_LINE_BYTES: usize = 64_000;
const MAX_GIT_OUTPUT_LINE_BYTES: usize = 64_000;
const MAX_EVIDENCE_ITEMS: usize = 12;
const MAX_MAP_EVIDENCE_ITEMS: usize = 16;
const MAX_NEXT_OBSERVATIONS: usize = 5;
const MAX_MAP_IMPORTANT_NEXT_OBSERVATIONS: usize = 4;
const MAX_SAMPLE_COMMITS: usize = 5;
const MAX_LOG_LINE_BYTES: usize = 64_000;
const MAX_PACKAGE_JSON_BYTES: u64 = 1_000_000;
const WORKSPACE_MAP_COMMAND: &str = "workspace map";
const WORKSPACE_STATUS_COMMAND: &str = "workspace status";
const WORKSPACE_DIFF_SUMMARY_COMMAND: &str = "workspace diff --summary";
const WORKSPACE_INDEX_STATUS_COMMAND: &str = "workspace index status";
const WORKSPACE_INDEX_COCHANGE_COMMAND: &str = "workspace index cochange";
const WORKSPACE_LOG_COMMAND: &str = "workspace log";
const WORKSPACE_RELATED_INDEX_COMMAND: &str = "workspace related <file> --by cochange --use-index";
const WORKSPACE_IMPACT_INDEX_COMMAND: &str = "workspace impact --diff --by cochange --use-index";
const WORKSPACE_IMPACT_COCHANGE_COMMAND: &str = "workspace impact --diff --by cochange";
const WORKSPACE_MAP_KIND: &str = "workspace_map";
const WORKSPACE_STATUS_KIND: &str = "workspace_status";
const WORKSPACE_SEARCH_KIND: &str = "workspace_search";
const WORKSPACE_INDEX_STATUS_KIND: &str = "workspace_index_status";
const WORKSPACE_INDEX_COCHANGE_KIND: &str = "workspace_index_cochange";
const WORKSPACE_RELATED_KIND: &str = "workspace_related";
const WORKSPACE_IMPACT_KIND: &str = "workspace_impact";
const WORKSPACE_READ_KIND: &str = "workspace_read";
const WORKSPACE_DIFF_KIND: &str = "workspace_diff";
const WORKSPACE_PATCH_KIND: &str = "workspace_patch";
const WORKSPACE_RUN_KIND: &str = "workspace_run";
const WORKSPACE_LOG_KIND: &str = "workspace_log";
const WORKSPACE_ROLLBACK_KIND: &str = "workspace_rollback";
const LOG_KIND_OBSERVE: &str = "observe";
const LOG_KIND_CHANGE: &str = "change";
const LOG_KIND_VERIFY: &str = "verify";
const LOG_OP_MAP: &str = "map";
const LOG_OP_STATUS: &str = "status";
const LOG_OP_SEARCH: &str = "search";
const LOG_OP_INDEX_STATUS: &str = "index status";
const LOG_OP_INDEX_COCHANGE: &str = "index cochange";
const LOG_OP_RELATED: &str = "related";
const LOG_OP_IMPACT: &str = "impact";
const LOG_OP_READ: &str = "read";
const LOG_OP_DIFF: &str = "diff";
const LOG_OP_PATCH: &str = "patch";
const LOG_OP_RUN: &str = "run";
const LOG_OP_ROLLBACK: &str = "rollback";
const IMPACT_SOURCE_DIFF: &str = "diff";
const RELATED_METHOD_COCHANGE: &str = "cochange";
const RANK_DIRECT: &str = "direct";
const RANK_PAGERANK: &str = "pagerank";
const RELATIONSHIP_SOURCE_COCHANGE_INDEX: &str = "cochange-index";
const RELATIONSHIP_SOURCE_GIT_LOG: &str = "git-log";
const RELATIONSHIP_SOURCE_RELATED_CLI: &str = "related-cli";
const EVIDENCE_REASON_GIT_DIFF_CHANGED_FILE: &str = "git diff changed file";
const EVIDENCE_REASON_PATCH_FILE_TARGET: &str = "patch file target";
const EVIDENCE_REASON_ROLLBACK_TARGET: &str = "rollback target";
const EVIDENCE_REASON_TEXT_MATCH: &str = "text match";
const EVIDENCE_REASON_REQUESTED_FILE_CONTENT: &str = "requested file content";
const IMPORTANT_REASON_CONFIGURATION_OR_PACKAGE_MANIFEST: &str =
    "configuration or package manifest";
const IMPORTANT_REASON_LIKELY_ENTRYPOINT: &str = "likely entrypoint";
const IMPORTANT_REASON_PRIMARY_PROJECT_DOCUMENTATION: &str = "primary project documentation";
const IMPORTANT_REASON_NO_LANGUAGE_SIGNALS: &str = "no language signals detected yet";
const INDEX_STATUS_FRESH: &str = "fresh";
const INDEX_STATUS_STALE: &str = "stale";
const INDEX_STATUS_MISSING: &str = "missing";
const INDEX_STATUS_INVALID: &str = "invalid";
const INDEX_STATUS_NOT_GIT_REPO: &str = "not_git_repo";
const SUMMARY_NOT_GIT_REPOSITORY: &str = "not a git repository";
const OUTPUT_TRUNCATED_MARKER: &str = "\n[output truncated]\n";
const INLINE_TRUNCATED_MARKER: &str = " [truncated]";
const SUMMARY_NOTE_MAP_TRUNCATED: &str = " (map truncated)";
const SUMMARY_NOTE_STATUS_TRUNCATED: &str = " (status truncated)";
const SUMMARY_NOTE_SEED_FILES_TRUNCATED: &str = " (seed files truncated)";
const SUMMARY_NOTE_FILES_TRUNCATED: &str = " (files truncated)";
const SUMMARY_NOTE_SUMMARY_AND_PATCH_TRUNCATED: &str = " (summary and patch truncated)";
const SUMMARY_NOTE_SUMMARY_TRUNCATED: &str = " (summary truncated)";
const SUMMARY_NOTE_PATCH_TRUNCATED: &str = " (patch truncated)";
const SUMMARY_NOTE_OUTPUT_TRUNCATED: &str = " (output truncated)";
const SUMMARY_NOTE_READ_TRUNCATED: &str = " (truncated)";
const SUMMARY_NOTE_OPERATION_LOG_UNREADABLE: &str = ", operation log unreadable";
const SUMMARY_NOTE_RECENT_OPERATIONS_TRUNCATED: &str = ", recent operations truncated";
const MAP_ENTRYPOINT_NAMES: &[&str] = &[
    "src/main.rs",
    "src/lib.rs",
    "src/index.ts",
    "src/main.ts",
    "src/index.js",
    "index.js",
    "main.go",
    "app.py",
    "main.py",
];
const MAP_CONFIG_NAMES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "tsconfig.json",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "Makefile",
    "justfile",
    ".env.example",
];
const MAP_STACK_ONLY_NAMES: &[&str] = &["pnpm-lock.yaml", "yarn.lock"];
static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Parser)]
#[command(name = "workspace")]
#[command(version)]
#[command(about = "Observable workspace operations for humans and LLM agents.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a semantic map of the current workspace.
    Map(MapArgs),
    /// Show git/workspace status and recent operations.
    Status(JsonArgs),
    /// Search text in the workspace using ripgrep.
    Search(SearchArgs),
    /// Build or refresh workspace indexes.
    Index(IndexArgs),
    /// Find related files using workspace signals such as git co-change history.
    Related(RelatedArgs),
    /// Estimate impacted files from current changes and workspace relationship signals.
    Impact(ImpactArgs),
    /// Read a text file, optionally by line range.
    Read(ReadArgs),
    /// Show the current git diff.
    Diff(DiffArgs),
    /// Apply a patch as a recorded transaction.
    Patch(PatchArgs),
    /// Run a command and record its output.
    Run(RunArgs),
    /// Show recorded workspace operations.
    Log(LogArgs),
    /// Roll back a patch transaction created by this CLI.
    Rollback(RollbackArgs),
}

#[derive(Args)]
struct JsonArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct MapArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Maximum directory depth to inspect for the structural map.
    #[arg(long, default_value_t = 3)]
    depth: usize,
    /// Include hidden files and directories except .git and .workspace.
    #[arg(long)]
    include_hidden: bool,
}

#[derive(Args)]
struct SearchArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Maximum matches to include.
    #[arg(long, default_value_t = 100)]
    max_results: usize,
    /// Search query passed to ripgrep.
    query: String,
}

#[derive(Args)]
struct IndexArgs {
    #[command(subcommand)]
    command: IndexCommands,
}

#[derive(Subcommand)]
enum IndexCommands {
    /// Show co-change index freshness and metadata.
    Status(IndexStatusArgs),
    /// Build a co-change graph index from git history.
    Cochange(IndexCochangeArgs),
}

#[derive(Args)]
struct IndexStatusArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct IndexCochangeArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Maximum recent commits to scan.
    #[arg(long, default_value_t = 1000)]
    max_commits: usize,
    /// Ignore broad commits above this changed-file count.
    #[arg(long, default_value_t = 40)]
    max_files_per_commit: usize,
}

#[derive(Args)]
struct RelatedArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Relationship signal to use.
    #[arg(long, value_enum, default_value_t = RelatedMethod::Cochange)]
    by: RelatedMethod,
    /// Maximum recent commits to scan.
    #[arg(long, default_value_t = 300)]
    max_commits: usize,
    /// Ignore broad commits above this changed-file count.
    #[arg(long, default_value_t = 40)]
    max_files_per_commit: usize,
    /// Maximum related files to include.
    #[arg(long, default_value_t = 10)]
    max_results: usize,
    /// Ranking algorithm. pagerank requires the co-change index.
    #[arg(long, value_enum, default_value_t = RankingMethod::Direct)]
    rank: RankingMethod,
    /// Use .workspace/index/cochange.json instead of scanning git log.
    #[arg(long)]
    use_index: bool,
    /// File path to use as the relationship seed.
    path: PathBuf,
}

#[derive(Args)]
struct ImpactArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Use current git working tree and index changes as seed files.
    #[arg(long)]
    diff: bool,
    /// Relationship signal to use.
    #[arg(long, value_enum, default_value_t = RelatedMethod::Cochange)]
    by: RelatedMethod,
    /// Maximum recent commits to scan.
    #[arg(long, default_value_t = 300)]
    max_commits: usize,
    /// Ignore broad commits above this changed-file count.
    #[arg(long, default_value_t = 40)]
    max_files_per_commit: usize,
    /// Maximum impacted files to include.
    #[arg(long, default_value_t = 20)]
    max_results: usize,
    /// Ranking algorithm. pagerank requires the co-change index.
    #[arg(long, value_enum, default_value_t = RankingMethod::Direct)]
    rank: RankingMethod,
    /// Use .workspace/index/cochange.json instead of scanning git log.
    #[arg(long)]
    use_index: bool,
}

#[derive(Clone, Debug, ValueEnum)]
enum RelatedMethod {
    Cochange,
}

impl RelatedMethod {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Cochange => RELATED_METHOD_COCHANGE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RankingMethod {
    Direct,
    Pagerank,
}

impl RankingMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => RANK_DIRECT,
            Self::Pagerank => RANK_PAGERANK,
        }
    }
}

#[derive(Args)]
struct ReadArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Read an inclusive line range such as 40:120.
    #[arg(long)]
    lines: Option<String>,
    /// File path to read. Relative paths are resolved from the workspace root.
    path: PathBuf,
}

#[derive(Args)]
struct DiffArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Show summary/stat output instead of the full patch.
    #[arg(long)]
    summary: bool,
}

#[derive(Args)]
struct PatchArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Optional human-readable transaction description.
    #[arg(long)]
    description: Option<String>,
    /// Patch file to apply with git apply.
    patch_file: PathBuf,
}

#[derive(Args)]
struct RunArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Command to execute through the platform shell.
    command: String,
}

#[derive(Args)]
struct LogArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Maximum number of log entries to show.
    #[arg(long, default_value_t = 30)]
    limit: usize,
}

#[derive(Args)]
struct RollbackArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Transaction id produced by workspace patch.
    transaction_id: String,
}

#[derive(Debug, Clone)]
struct Workspace {
    root: PathBuf,
    is_git_repo: bool,
}

#[derive(Serialize)]
struct Observation<T: Serialize> {
    kind: String,
    scope: String,
    summary: String,
    data: T,
    evidence: Vec<Evidence>,
    truncated: bool,
    next_observations: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Evidence {
    path: String,
    lines: Option<String>,
    reason: String,
}

#[derive(Serialize, Clone)]
struct GitSummary {
    is_repo: bool,
    branch: Option<String>,
    dirty_file_count: usize,
    untracked_file_count: usize,
    dirty_files: Vec<String>,
    untracked_files: Vec<String>,
    omitted_dirty_files: usize,
    omitted_untracked_files: usize,
}

impl GitSummary {
    fn omitted_files(&self) -> bool {
        self.omitted_dirty_files > 0 || self.omitted_untracked_files > 0
    }
}

#[derive(Serialize)]
struct WorkspaceMap {
    root: String,
    git: GitSummary,
    stack: StackSummary,
    structure: StructureSummary,
    commands: BTreeMap<String, String>,
    stats: WorkspaceStats,
    important_files: Vec<ImportantFile>,
    recent_files: Vec<String>,
    omitted: MapOmittedCounts,
}

#[derive(Serialize)]
struct StackSummary {
    languages: Vec<String>,
    package_managers: Vec<String>,
    frameworks: Vec<String>,
}

#[derive(Serialize)]
struct StructureSummary {
    directories: Vec<String>,
    entrypoints: Vec<String>,
    tests: Vec<String>,
    configs: Vec<String>,
    docs: Vec<String>,
}

#[derive(Serialize)]
struct WorkspaceStats {
    file_count: usize,
    directory_count: usize,
    large_files: Vec<LargeFile>,
}

#[derive(Serialize)]
struct LargeFile {
    path: String,
    bytes: u64,
}

#[derive(Serialize, Default)]
struct MapOmittedCounts {
    directories: usize,
    entrypoints: usize,
    tests: usize,
    configs: usize,
    docs: usize,
    large_files: usize,
}

impl MapOmittedCounts {
    fn any(&self) -> bool {
        self.directories > 0
            || self.entrypoints > 0
            || self.tests > 0
            || self.configs > 0
            || self.docs > 0
            || self.large_files > 0
    }
}

#[derive(Serialize)]
struct ImportantFile {
    path: String,
    reason: String,
}

#[derive(Default)]
struct MapSignals {
    languages: BTreeSet<String>,
    named_files: BTreeSet<String>,
    directories: BoundedMapItems,
    tests: BoundedMapItems,
    config_extras: BoundedMapItems,
    docs: BoundedMapItems,
}

#[derive(Default)]
struct BoundedMapItems {
    items: BTreeSet<String>,
    total_items: usize,
}

impl BoundedMapItems {
    fn push(&mut self, item: String) {
        if !self.items.insert(item) {
            return;
        }
        self.total_items += 1;
        if self.items.len() > MAX_MAP_LIST_ITEMS
            && let Some(last) = self.items.iter().next_back().cloned()
        {
            self.items.remove(&last);
        }
    }

    fn observed(&self) -> Vec<String> {
        self.items.iter().cloned().collect()
    }

    fn total_items(&self) -> usize {
        self.total_items
    }

    fn omitted_count(&self) -> usize {
        self.total_items.saturating_sub(self.items.len())
    }
}

#[derive(Serialize)]
struct StatusData {
    root: String,
    git: GitSummary,
    index_status: IndexStatusData,
    recent_operations: Vec<LogEntry>,
    recent_operations_omitted: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    recent_operations_error: Option<String>,
}

struct StatusRecentOperations {
    entries: Vec<LogEntry>,
    omitted_lines: usize,
    error: Option<String>,
}

#[derive(Serialize)]
struct SearchData {
    query: String,
    total_matches: usize,
    truncated_match_texts: usize,
    matches: Vec<SearchMatch>,
}

#[derive(Serialize)]
struct SearchMatch {
    path: String,
    line: u64,
    column: u64,
    text: String,
}

struct FallbackSearchResult {
    matches: Vec<SearchMatch>,
    total_matches: usize,
    truncated_match_texts: usize,
}

struct FallbackLineSearch {
    line_number: u64,
    byte_offset: usize,
    scan_tail: Vec<u8>,
    matched: bool,
    match_column: u64,
    display_text: String,
    display_char_count: usize,
    display_truncated: bool,
    capture_display: bool,
    pending_utf8: Vec<u8>,
    pending_line_cr: bool,
    saw_bytes: bool,
}

impl FallbackLineSearch {
    fn with_display(line_number: u64, capture_display: bool) -> Self {
        Self {
            line_number,
            byte_offset: 0,
            scan_tail: Vec::new(),
            matched: false,
            match_column: 0,
            display_text: String::new(),
            display_char_count: 0,
            display_truncated: false,
            capture_display,
            pending_utf8: Vec::new(),
            pending_line_cr: false,
            saw_bytes: false,
        }
    }
}

#[derive(Serialize)]
struct RelatedData {
    target: String,
    method: String,
    ranking: String,
    relationship_source: String,
    is_repo: bool,
    commits_scanned: usize,
    commits_matched: usize,
    ignored_large_commits: usize,
    max_commits: usize,
    max_files_per_commit: usize,
    related: Vec<RelatedFile>,
}

struct RelatedDataMetadata {
    target: String,
    relationship: RelationshipMetadata,
}

impl RelatedDataMetadata {
    fn new(
        target: &str,
        method: &RelatedMethod,
        rank: RankingMethod,
        relationship_source: impl Into<String>,
        is_repo: bool,
    ) -> Self {
        Self {
            target: target.to_string(),
            relationship: RelationshipMetadata::new(method, rank, relationship_source, is_repo),
        }
    }

    fn cochange(
        target: &str,
        rank: RankingMethod,
        relationship_source: impl Into<String>,
        is_repo: bool,
    ) -> Self {
        Self::new(
            target,
            &RelatedMethod::Cochange,
            rank,
            relationship_source,
            is_repo,
        )
    }
}

struct RelationshipMetadata {
    method: String,
    ranking: String,
    relationship_source: String,
    is_repo: bool,
}

impl RelationshipMetadata {
    fn new(
        method: &RelatedMethod,
        rank: RankingMethod,
        relationship_source: impl Into<String>,
        is_repo: bool,
    ) -> Self {
        Self {
            method: method.as_str().to_string(),
            ranking: rank.as_str().to_string(),
            relationship_source: relationship_source.into(),
            is_repo,
        }
    }

    fn into_parts(self) -> (String, String, String, bool) {
        (
            self.method,
            self.ranking,
            self.relationship_source,
            self.is_repo,
        )
    }
}

#[derive(Clone, Copy)]
struct RelationshipStats {
    commits_scanned: usize,
    commits_matched: usize,
    ignored_large_commits: usize,
}

impl RelationshipStats {
    fn new(commits_scanned: usize, commits_matched: usize, ignored_large_commits: usize) -> Self {
        Self {
            commits_scanned,
            commits_matched,
            ignored_large_commits,
        }
    }

    fn none() -> Self {
        Self::new(0, 0, 0)
    }

    fn from_cochange_index(index: &CochangeIndex, commits_matched: usize) -> Self {
        Self::new(
            index.commits_scanned,
            commits_matched,
            index.ignored_large_commits,
        )
    }

    fn from_git_log(
        commits: &[GitCommitFiles],
        commits_matched: usize,
        ignored_large_commits: usize,
    ) -> Self {
        Self::new(commits.len(), commits_matched, ignored_large_commits)
    }

    fn from_related_cli(commits_matched: usize) -> Self {
        Self::new(0, commits_matched, 0)
    }

    fn into_parts(self) -> (usize, usize, usize) {
        (
            self.commits_scanned,
            self.commits_matched,
            self.ignored_large_commits,
        )
    }
}

#[derive(Clone, Copy)]
struct RelationshipLimits {
    max_commits: usize,
    max_files_per_commit: usize,
}

impl RelationshipLimits {
    fn new(max_commits: usize, max_files_per_commit: usize) -> Self {
        Self {
            max_commits,
            max_files_per_commit,
        }
    }

    fn from_options(max_commits: usize, max_files_per_commit: usize) -> Self {
        Self::new(max_commits, max_files_per_commit)
    }

    fn from_cochange_index(index: &CochangeIndex) -> Self {
        Self::new(index.max_commits, index.max_files_per_commit)
    }

    fn into_parts(self) -> (usize, usize) {
        (self.max_commits, self.max_files_per_commit)
    }
}

#[derive(Serialize, Clone)]
struct RelatedFile {
    path: String,
    score: f64,
    cochanged_commits: usize,
    weighted_cochanges: f64,
    sample_commits: Vec<String>,
}

#[derive(Serialize)]
struct ImpactData {
    source: String,
    method: String,
    ranking: String,
    relationship_source: String,
    is_repo: bool,
    seed_files: Vec<String>,
    seed_file_count: usize,
    omitted_seed_files: usize,
    commits_scanned: usize,
    commits_matched: usize,
    ignored_large_commits: usize,
    max_commits: usize,
    max_files_per_commit: usize,
    impacted: Vec<ImpactFile>,
}

struct ImpactDataMetadata {
    source: String,
    relationship: RelationshipMetadata,
}

impl ImpactDataMetadata {
    fn new(
        method: &RelatedMethod,
        rank: RankingMethod,
        relationship_source: impl Into<String>,
        is_repo: bool,
    ) -> Self {
        Self {
            source: IMPACT_SOURCE_DIFF.to_string(),
            relationship: RelationshipMetadata::new(method, rank, relationship_source, is_repo),
        }
    }

    fn cochange(
        rank: RankingMethod,
        relationship_source: impl Into<String>,
        is_repo: bool,
    ) -> Self {
        Self::new(&RelatedMethod::Cochange, rank, relationship_source, is_repo)
    }
}

struct SeedFileSummary {
    seed_files: Vec<String>,
    seed_file_count: usize,
    omitted_seed_files: usize,
}

impl SeedFileSummary {
    fn empty() -> Self {
        Self {
            seed_files: vec![],
            seed_file_count: 0,
            omitted_seed_files: 0,
        }
    }

    fn from_seed_files(seed_files: &[String], max_seed_files: usize) -> Self {
        let (observed_seed_files, omitted_seed_files) =
            observed_string_prefix(seed_files, max_seed_files);
        Self {
            seed_files: observed_seed_files,
            seed_file_count: seed_files.len(),
            omitted_seed_files,
        }
    }
}

#[derive(Serialize, Clone)]
struct ImpactFile {
    path: String,
    score: f64,
    cochanged_commits: usize,
    weighted_cochanges: f64,
    seed_files: Vec<String>,
    sample_commits: Vec<String>,
}

struct ObservedRelated {
    target: String,
    data: RelatedData,
}

#[derive(Default)]
struct RelatedCliImpactAccumulator {
    score: f64,
    cochanged_commits: usize,
    weighted_cochanges: f64,
    seed_files: BTreeSet<String>,
    sample_commits: Vec<String>,
}

#[derive(Serialize)]
struct ReadData {
    path: String,
    lines: Option<String>,
    content: String,
}

struct ReadContent {
    content: String,
    truncated: bool,
}

struct ObservedRead {
    data: ReadData,
    content_truncated: bool,
}

#[derive(Serialize)]
struct DiffData {
    is_repo: bool,
    summary: String,
    file_count: usize,
    files: Vec<String>,
    omitted_files: usize,
    patch: Option<String>,
}

struct ObservedDiff {
    data: DiffData,
    summary_truncated: bool,
    patch_truncated: bool,
}

#[derive(Serialize)]
struct PatchData {
    transaction_id: String,
    patch_file: String,
    stored_patch: String,
    file_count: usize,
    files_changed: Vec<String>,
    omitted_files: usize,
}

#[derive(Serialize)]
struct RunData {
    command: String,
    cwd: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
}

struct ObservedRun {
    data: RunData,
    output_truncated: bool,
}

struct CapturedOutput {
    text: String,
    truncated: bool,
}

struct CapturedCommandOutput {
    status: std::process::ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

type CapturedOutputReader = std::thread::JoinHandle<Result<CapturedOutput>>;

#[derive(Deserialize, Serialize, Clone)]
struct LogEntry {
    id: String,
    timestamp_unix_ms: u128,
    kind: String,
    op: String,
    scope: String,
    summary: String,
    transaction_id: Option<String>,
}

#[derive(Serialize)]
struct LogData {
    log_path: String,
    omitted_lines: usize,
    entries: Vec<LogEntry>,
}

#[derive(Default)]
struct LogWindow {
    entries: Vec<LogEntry>,
    omitted_lines: usize,
}

struct StoredLogLine {
    line_number: usize,
    bytes: Vec<u8>,
    oversized: bool,
}

struct PendingLogLine {
    line_number: usize,
    bytes: Vec<u8>,
    oversized: bool,
    saw_non_whitespace: bool,
}

impl PendingLogLine {
    fn new(line_number: usize) -> Self {
        Self {
            line_number,
            bytes: Vec::new(),
            oversized: false,
            saw_non_whitespace: false,
        }
    }

    fn push_segment(&mut self, segment: &[u8]) {
        if segment.iter().any(|byte| !byte.is_ascii_whitespace()) {
            self.saw_non_whitespace = true;
        }

        let remaining = MAX_LOG_LINE_BYTES.saturating_sub(self.bytes.len());
        if remaining > 0 {
            let bytes_to_store = remaining.min(segment.len());
            self.bytes.extend_from_slice(&segment[..bytes_to_store]);
        }
        if segment.len() > remaining {
            self.oversized = true;
        }
    }

    fn into_stored(mut self) -> StoredLogLine {
        if !self.oversized && self.bytes.last() == Some(&b'\r') {
            self.bytes.pop();
        }
        StoredLogLine {
            line_number: self.line_number,
            bytes: self.bytes,
            oversized: self.oversized,
        }
    }
}

#[derive(Debug)]
struct RipgrepJsonLineTooLarge {
    line_number: usize,
    max_bytes: usize,
}

impl std::fmt::Display for RipgrepJsonLineTooLarge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "ripgrep JSON line {} exceeded {} bytes",
            self.line_number, self.max_bytes
        )
    }
}

impl std::error::Error for RipgrepJsonLineTooLarge {}

struct BoundedOutputLine {
    line_number: usize,
    bytes: Vec<u8>,
    exceeded: bool,
}

struct BoundedFileList {
    files: Vec<String>,
    total_files: usize,
    omitted_files: usize,
}

impl BoundedFileList {
    fn empty() -> Self {
        Self {
            files: vec![],
            total_files: 0,
            omitted_files: 0,
        }
    }
}

struct BoundedPathAccumulator {
    paths: Vec<String>,
    total_count: usize,
    max_paths: usize,
}

impl BoundedPathAccumulator {
    fn new(max_paths: usize) -> Self {
        Self {
            paths: Vec::new(),
            total_count: 0,
            max_paths,
        }
    }

    fn push(&mut self, path: String) {
        self.total_count += 1;
        if self.paths.len() < self.max_paths {
            self.paths.push(path);
        }
    }

    fn omitted_count(&self) -> usize {
        self.total_count.saturating_sub(self.paths.len())
    }

    fn into_file_list(self) -> BoundedFileList {
        let omitted_files = self.omitted_count();
        BoundedFileList {
            files: self.paths,
            total_files: self.total_count,
            omitted_files,
        }
    }
}

#[derive(Serialize)]
struct RollbackData {
    transaction_id: String,
    rollback_transaction_id: String,
    stored_patch: String,
    file_count: usize,
    files_changed: Vec<String>,
    omitted_files: usize,
}

#[derive(Serialize)]
struct IndexStatusData {
    is_repo: bool,
    path: String,
    exists: bool,
    readable: bool,
    status: String,
    fresh: bool,
    current_head: Option<String>,
    index_head: Option<String>,
    generated_at_unix_ms: Option<u128>,
    max_commits: Option<usize>,
    max_files_per_commit: Option<usize>,
    commits_scanned: Option<usize>,
    commits_indexed: Option<usize>,
    ignored_large_commits: Option<usize>,
    file_count: Option<usize>,
    edge_count: Option<usize>,
    error: Option<String>,
}

#[derive(Serialize)]
struct IndexCochangeData {
    path: String,
    version: u32,
    generated_at_unix_ms: u128,
    head: Option<String>,
    max_commits: usize,
    max_files_per_commit: usize,
    commits_scanned: usize,
    commits_indexed: usize,
    ignored_large_commits: usize,
    file_count: usize,
    edge_count: usize,
}

#[derive(Deserialize, Serialize, Clone)]
struct CochangeIndex {
    version: u32,
    generated_at_unix_ms: u128,
    head: Option<String>,
    max_commits: usize,
    max_files_per_commit: usize,
    commits_scanned: usize,
    commits_indexed: usize,
    ignored_large_commits: usize,
    file_commit_counts: BTreeMap<String, usize>,
    edges: Vec<CochangeEdge>,
}

#[derive(Deserialize, Serialize, Clone)]
struct CochangeEdge {
    a: String,
    b: String,
    cochanged_commits: usize,
    weighted_cochanges: f64,
    sample_commits: Vec<String>,
}

#[derive(Clone, Debug)]
struct GitCommitFiles {
    hash: String,
    files: Vec<String>,
}

#[derive(Default)]
struct GitLogNameOnlyState {
    commits: Vec<GitCommitFiles>,
    current_hash: Option<String>,
    current_files: BTreeSet<String>,
}

impl GitLogNameOnlyState {
    fn push_line(&mut self, line: &str) {
        if let Some(hash) = line.strip_prefix("commit:") {
            self.push_current_commit();
            self.current_hash = Some(hash.trim().to_string());
            return;
        }

        if let Some(path) = git_name_only_path(line)
            && should_include_repo_file(&path)
        {
            self.current_files.insert(path);
        }
    }

    fn finish(mut self) -> Vec<GitCommitFiles> {
        self.push_current_commit();
        self.commits
    }

    fn push_current_commit(&mut self) {
        if let Some(hash) = self.current_hash.take() {
            self.commits.push(GitCommitFiles {
                hash,
                files: std::mem::take(&mut self.current_files)
                    .into_iter()
                    .collect(),
            });
        }
    }
}

#[derive(Default)]
struct CochangeAccumulator {
    cochanged_commits: usize,
    weighted_cochanges: f64,
    sample_commits: Vec<String>,
}

struct CochangeRanking {
    related: Vec<RelatedFile>,
    commits_matched: usize,
    ignored_large_commits: usize,
}

#[derive(Default)]
struct ImpactAccumulator {
    cochanged_commits: usize,
    weighted_cochanges: f64,
    seed_files: BTreeSet<String>,
    sample_commits: Vec<String>,
}

struct ImpactRanking {
    impacted: Vec<ImpactFile>,
    commits_matched: usize,
    ignored_large_commits: usize,
}

struct PageRankHit {
    path: String,
    score: f64,
}

#[derive(Default)]
struct CochangeEdgeAccumulator {
    cochanged_commits: usize,
    weighted_cochanges: f64,
    sample_commits: Vec<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace = Workspace::detect()?;

    match cli.command {
        Commands::Map(args) => cmd_map(&workspace, args),
        Commands::Status(args) => cmd_status(&workspace, args),
        Commands::Search(args) => cmd_search(&workspace, args),
        Commands::Index(args) => cmd_index(&workspace, args),
        Commands::Related(args) => cmd_related(&workspace, args),
        Commands::Impact(args) => cmd_impact(&workspace, args),
        Commands::Read(args) => cmd_read(&workspace, args),
        Commands::Diff(args) => cmd_diff(&workspace, args),
        Commands::Patch(args) => cmd_patch(&workspace, args),
        Commands::Run(args) => cmd_run(&workspace, args),
        Commands::Log(args) => cmd_log(&workspace, args),
        Commands::Rollback(args) => cmd_rollback(&workspace, args),
    }
}

impl Workspace {
    fn detect() -> Result<Self> {
        let cwd = std::env::current_dir().context("failed to read current directory")?;
        let mut cursor = Some(cwd.as_path());

        while let Some(path) = cursor {
            if path.join(".git").exists() {
                return Ok(Self {
                    root: path.to_path_buf(),
                    is_git_repo: true,
                });
            }
            cursor = path.parent();
        }

        Ok(Self {
            root: cwd.clone(),
            is_git_repo: false,
        })
    }

    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn resolve_existing_workspace_path(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.resolve_path(path);
        let canonical_root = self
            .root
            .canonicalize()
            .with_context(|| format!("failed to resolve workspace root {}", self.root.display()))?;
        let canonical_path = resolved
            .canonicalize()
            .with_context(|| format!("failed to resolve path {}", resolved.display()))?;

        if !canonical_path.starts_with(&canonical_root) {
            bail!(
                "path {} is outside workspace root {}",
                canonical_path.display(),
                canonical_root.display()
            );
        }

        Ok(canonical_path)
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn log_path(&self) -> PathBuf {
        self.root.join(LOG_FILE)
    }

    fn transaction_dir(&self) -> PathBuf {
        self.root.join(TRANSACTION_DIR)
    }

    fn cochange_index_path(&self) -> PathBuf {
        self.root.join(COCHANGE_INDEX_FILE)
    }
}

fn cmd_map(workspace: &Workspace, args: MapArgs) -> Result<()> {
    let map = observed_map(workspace, &args)?;
    let observation = map_observation(map);
    output_best_effort_logged_observation(workspace, args.json, LOG_OP_MAP, &observation, print_map)
}

fn cmd_status(workspace: &Workspace, args: JsonArgs) -> Result<()> {
    let data = observed_status(workspace)?;
    let observation = status_observation(data);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_STATUS,
        &observation,
        print_status,
    )
}

fn cmd_search(workspace: &Workspace, args: SearchArgs) -> Result<()> {
    let data = observed_search(workspace, &args)?;
    let observation = search_observation(workspace, data);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_SEARCH,
        &observation,
        print_search,
    )
}

fn cmd_index(workspace: &Workspace, args: IndexArgs) -> Result<()> {
    match args.command {
        IndexCommands::Status(args) => cmd_index_status(workspace, args),
        IndexCommands::Cochange(args) => cmd_index_cochange(workspace, args),
    }
}

fn cmd_index_status(workspace: &Workspace, args: IndexStatusArgs) -> Result<()> {
    let data = observed_index_status(workspace);
    let observation = index_status_observation(data);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_INDEX_STATUS,
        &observation,
        print_index_status,
    )
}

fn cmd_index_cochange(workspace: &Workspace, args: IndexCochangeArgs) -> Result<()> {
    let data = observed_index_cochange(workspace, &args)?;
    let observation = index_cochange_observation(data);
    output_required_logged_observation(
        workspace,
        args.json,
        LOG_OP_INDEX_COCHANGE,
        &observation,
        print_index_cochange,
    )
}

fn cmd_related(workspace: &Workspace, args: RelatedArgs) -> Result<()> {
    let related = observed_related_args(workspace, &args)?;
    let observation = related_observation(workspace, &related.target, related.data);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_RELATED,
        &observation,
        print_related,
    )
}

fn cmd_impact(workspace: &Workspace, args: ImpactArgs) -> Result<()> {
    let data = observed_impact_args(workspace, &args)?;
    let observation = impact_observation(workspace, data);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_IMPACT,
        &observation,
        print_impact,
    )
}

fn cmd_read(workspace: &Workspace, args: ReadArgs) -> Result<()> {
    let read = observed_read_args(workspace, &args)?;
    let observation = read_observation(read);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_READ,
        &observation,
        print_read,
    )
}

fn cmd_diff(workspace: &Workspace, args: DiffArgs) -> Result<()> {
    let diff = observed_diff(workspace, args.summary)?;
    let observation = diff_observation(workspace, diff);
    output_best_effort_logged_observation(
        workspace,
        args.json,
        LOG_OP_DIFF,
        &observation,
        print_diff,
    )
}

fn cmd_patch(workspace: &Workspace, args: PatchArgs) -> Result<()> {
    let patch = apply_patch_transaction(workspace, &args.patch_file)?;
    let observation = patch_transaction_observation(workspace, &patch);
    let log_summary = patch_log_summary(args.description, &observation);

    output_changed_observation_with_summary(
        workspace,
        args.json,
        LOG_OP_PATCH,
        &log_summary,
        &patch.transaction_id,
        &observation,
        print_patch,
    )
}

fn apply_patch_transaction(
    workspace: &Workspace,
    patch_file: &Path,
) -> Result<AppliedPatchTransaction> {
    let patch_path = workspace.resolve_existing_workspace_path(patch_file)?;
    let files_changed = extract_patch_files_from_path(&patch_path)
        .with_context(|| format!("failed to read patch {}", patch_path.display()))?;
    validate_patch_targets(&files_changed)?;
    run_git_apply(workspace, &patch_path, ["--check"])?;
    ensure_log_writable(workspace)?;

    let transaction_id = new_id("tx");
    let stored_patch = store_transaction_patch_for_id(workspace, &transaction_id, &patch_path)?;
    if let Err(error) = run_git_apply(workspace, &patch_path, []) {
        let _ = fs::remove_file(&stored_patch);
        return Err(error);
    }

    Ok(AppliedPatchTransaction {
        transaction_id,
        patch_path,
        stored_patch,
        files_changed,
    })
}

fn store_transaction_patch_for_id(
    workspace: &Workspace,
    transaction_id: &str,
    patch_path: &Path,
) -> Result<PathBuf> {
    let transaction_dir = workspace.transaction_dir();
    fs::create_dir_all(&transaction_dir).with_context(|| {
        format!(
            "failed to create transaction directory {}",
            transaction_dir.display()
        )
    })?;
    let stored_patch = transaction_dir.join(format!("{transaction_id}.patch"));
    store_transaction_patch(patch_path, &stored_patch)?;
    Ok(stored_patch)
}

fn store_transaction_patch(source: &Path, destination: &Path) -> Result<()> {
    let temp_path = temp_sibling_path(destination, "patch-store")?;
    copy_file_to_temp_path(source, &temp_path)?;
    if let Err(error) = fs::rename(&temp_path, destination)
        .with_context(|| format!("failed to store patch {}", destination.display()))
    {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

fn copy_file_to_temp_path(source: &Path, temp_path: &Path) -> Result<u64> {
    let source_file = fs::File::open(source)
        .with_context(|| format!("failed to read patch {}", source.display()))?;
    let temp_file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
    {
        Ok(file) => file,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create stored patch {}", temp_path.display()));
        }
    };

    let result = copy_file_contents(source_file, temp_file, temp_path);
    if result.is_err() {
        let _ = fs::remove_file(temp_path);
    }
    result
}

fn copy_file_contents(source_file: fs::File, temp_file: fs::File, temp_path: &Path) -> Result<u64> {
    let mut reader = BufReader::new(source_file);
    let mut writer = BufWriter::new(temp_file);
    let bytes_copied = std::io::copy(&mut reader, &mut writer)
        .with_context(|| format!("failed to copy stored patch {}", temp_path.display()))?;
    writer
        .flush()
        .with_context(|| format!("failed to flush stored patch {}", temp_path.display()))?;
    let file = writer
        .into_inner()
        .with_context(|| format!("failed to finish stored patch {}", temp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync stored patch {}", temp_path.display()))?;
    Ok(bytes_copied)
}

fn cmd_run(workspace: &Workspace, args: RunArgs) -> Result<()> {
    ensure_log_writable(workspace)?;
    let run = execute_run_command(workspace, &args.command)?;
    let observation = run_observation(run);

    output_verified_observation(workspace, args.json, LOG_OP_RUN, &observation, print_run)
}

fn execute_run_command(workspace: &Workspace, command_text: &str) -> Result<ObservedRun> {
    let start = Instant::now();
    let mut command = shell_command(command_text);
    let mut child = command
        .current_dir(&workspace.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run command {command_text:?}"))?;
    let stdout_reader = capture_child_stdout(&mut child, "command stdout", MAX_CAPTURED_OUTPUT)?;
    let stderr_reader = capture_child_stderr(&mut child, "command stderr", MAX_CAPTURED_OUTPUT)?;
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for command {command_text:?}"))?;
    let duration_ms = start.elapsed().as_millis();
    let stdout = join_captured_output_reader(stdout_reader, "stdout")?;
    let stderr = join_captured_output_reader(stderr_reader, "stderr")?;
    Ok(observed_run(
        workspace,
        command_text,
        status.code(),
        duration_ms,
        stdout,
        stderr,
    ))
}

fn cmd_log(workspace: &Workspace, args: LogArgs) -> Result<()> {
    let data = observed_log(workspace, &args)?;
    let observation = log_observation(data);
    output_observation(args.json, &observation, print_log)
}

fn cmd_rollback(workspace: &Workspace, args: RollbackArgs) -> Result<()> {
    let rollback = apply_rollback_transaction(workspace, &args.transaction_id)?;
    let observation = rollback_transaction_observation(workspace, &args.transaction_id, &rollback);

    output_changed_observation(
        workspace,
        args.json,
        LOG_OP_ROLLBACK,
        &rollback.rollback_transaction_id,
        &observation,
        print_rollback,
    )
}

fn apply_rollback_transaction(
    workspace: &Workspace,
    transaction_id: &str,
) -> Result<AppliedRollbackTransaction> {
    let stored_patch = transaction_patch_path(workspace, transaction_id)?;
    if !stored_patch.exists() {
        bail!(
            "transaction patch not found: {}",
            workspace.relative(&stored_patch)
        );
    }

    let files_changed = extract_patch_files_from_path(&stored_patch)
        .with_context(|| format!("failed to read stored patch {}", stored_patch.display()))?;
    run_git_apply(workspace, &stored_patch, ["--reverse", "--check"])?;
    ensure_log_writable(workspace)?;
    run_git_apply(workspace, &stored_patch, ["--reverse"])?;

    Ok(AppliedRollbackTransaction {
        rollback_transaction_id: new_id("rb"),
        stored_patch,
        files_changed,
    })
}

struct ObservedChangedFiles {
    files: Vec<String>,
    file_count: usize,
    omitted_files: usize,
}

struct AppliedPatchTransaction {
    transaction_id: String,
    patch_path: PathBuf,
    stored_patch: PathBuf,
    files_changed: Vec<String>,
}

struct AppliedRollbackTransaction {
    rollback_transaction_id: String,
    stored_patch: PathBuf,
    files_changed: Vec<String>,
}

fn patch_transaction_observation(
    workspace: &Workspace,
    patch: &AppliedPatchTransaction,
) -> Observation<PatchData> {
    let data = patch_data(
        workspace,
        &patch.transaction_id,
        &patch.patch_path,
        &patch.stored_patch,
        &patch.files_changed,
    );
    patch_observation(data, &patch.files_changed)
}

fn rollback_transaction_observation(
    workspace: &Workspace,
    transaction_id: &str,
    rollback: &AppliedRollbackTransaction,
) -> Observation<RollbackData> {
    let data = rollback_data(
        workspace,
        transaction_id,
        &rollback.rollback_transaction_id,
        &rollback.stored_patch,
        &rollback.files_changed,
    );
    rollback_observation(data, &rollback.files_changed)
}

fn patch_log_summary(description: Option<String>, observation: &Observation<PatchData>) -> String {
    description.unwrap_or_else(|| observation.summary.clone())
}

fn observed_changed_files(files_changed: &[String]) -> ObservedChangedFiles {
    let mut files = files_changed.to_vec();
    let omitted_files = truncate_vec(&mut files, MAX_CHANGED_FILES);
    ObservedChangedFiles {
        files,
        file_count: files_changed.len(),
        omitted_files,
    }
}

fn patch_data(
    workspace: &Workspace,
    transaction_id: &str,
    patch_path: &Path,
    stored_patch: &Path,
    files_changed: &[String],
) -> PatchData {
    let observed_files = observed_changed_files(files_changed);
    PatchData {
        transaction_id: transaction_id.to_string(),
        patch_file: workspace.relative(patch_path),
        stored_patch: workspace.relative(stored_patch),
        file_count: observed_files.file_count,
        files_changed: observed_files.files,
        omitted_files: observed_files.omitted_files,
    }
}

fn patch_observation(data: PatchData, files_changed: &[String]) -> Observation<PatchData> {
    let summary = transaction_file_summary(
        "applied patch",
        &data.transaction_id,
        data.file_count,
        data.omitted_files,
    );
    let evidence = changed_file_evidence(files_changed, EVIDENCE_REASON_PATCH_FILE_TARGET);
    let next_observations = patch_followup_observations(&data.transaction_id);
    let truncated = transaction_files_truncated(data.omitted_files);
    observation_with_evidence(
        WORKSPACE_PATCH_KIND,
        data.patch_file.clone(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    )
}

fn rollback_data(
    workspace: &Workspace,
    transaction_id: &str,
    rollback_transaction_id: &str,
    stored_patch: &Path,
    files_changed: &[String],
) -> RollbackData {
    let observed_files = observed_changed_files(files_changed);
    RollbackData {
        transaction_id: transaction_id.to_string(),
        rollback_transaction_id: rollback_transaction_id.to_string(),
        stored_patch: workspace.relative(stored_patch),
        file_count: observed_files.file_count,
        files_changed: observed_files.files,
        omitted_files: observed_files.omitted_files,
    }
}

fn rollback_observation(data: RollbackData, files_changed: &[String]) -> Observation<RollbackData> {
    let summary = transaction_file_summary(
        "rolled back",
        &data.transaction_id,
        data.file_count,
        data.omitted_files,
    );
    let evidence = changed_file_evidence(files_changed, EVIDENCE_REASON_ROLLBACK_TARGET);
    let next_observations = rollback_followup_observations();
    let truncated = transaction_files_truncated(data.omitted_files);
    observation_with_evidence(
        WORKSPACE_ROLLBACK_KIND,
        data.transaction_id.clone(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    )
}

fn transaction_files_truncated(omitted_files: usize) -> bool {
    omitted_files > 0
}

fn changed_file_evidence(files_changed: &[String], reason: &str) -> Vec<Evidence> {
    files_changed
        .iter()
        .take(MAX_CHANGED_FILES)
        .map(|path| Evidence {
            path: path.clone(),
            lines: None,
            reason: reason.to_string(),
        })
        .collect()
}

fn read_next_observations<'a, I>(workspace: &Workspace, paths: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    paths
        .into_iter()
        .filter(|path| workspace.resolve_path(Path::new(path)).is_file())
        .take(MAX_NEXT_OBSERVATIONS)
        .map(workspace_read_command)
        .collect()
}

fn search_evidence(matches: &[SearchMatch]) -> Vec<Evidence> {
    matches
        .iter()
        .take(MAX_EVIDENCE_ITEMS)
        .map(|item| Evidence {
            path: item.path.clone(),
            lines: Some(item.line.to_string()),
            reason: EVIDENCE_REASON_TEXT_MATCH.to_string(),
        })
        .collect()
}

fn search_data(
    query: &str,
    matches: Vec<SearchMatch>,
    total_matches: usize,
    truncated_match_texts: usize,
) -> SearchData {
    SearchData {
        query: query.to_string(),
        total_matches,
        truncated_match_texts,
        matches,
    }
}

fn observed_search(workspace: &Workspace, args: &SearchArgs) -> Result<SearchData> {
    let (matches, total_matches, truncated_match_texts) =
        rg_search(workspace, &args.query, args.max_results)?;
    Ok(search_data(
        &args.query,
        matches,
        total_matches,
        truncated_match_texts,
    ))
}

fn search_observation(workspace: &Workspace, data: SearchData) -> Observation<SearchData> {
    let summary = search_summary(&data);
    let evidence = search_evidence(&data.matches);
    let truncated = search_truncated(&data);
    let next_observations = search_next_observations(&data.matches);
    observation_with_evidence(
        WORKSPACE_SEARCH_KIND,
        workspace.root.to_string_lossy().into_owned(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    )
}

fn search_next_observations(matches: &[SearchMatch]) -> Vec<String> {
    matches
        .iter()
        .take(MAX_NEXT_OBSERVATIONS)
        .map(|item| workspace_read_lines_command(&item.path, item.line, item.line))
        .collect()
}

fn read_evidence(data: &ReadData) -> Vec<Evidence> {
    vec![Evidence {
        path: data.path.clone(),
        lines: data.lines.clone(),
        reason: EVIDENCE_REASON_REQUESTED_FILE_CONTENT.to_string(),
    }]
}

fn read_line_label(range: Option<(usize, usize)>) -> Option<String> {
    range.map(|(start, end)| format!("{start}:{end}"))
}

fn read_data(path: String, lines: Option<String>, content: String) -> ReadData {
    ReadData {
        path,
        lines,
        content,
    }
}

fn observed_read(
    workspace: &Workspace,
    path: &Path,
    range: Option<(usize, usize)>,
) -> Result<ObservedRead> {
    let line_label = read_line_label(range);
    let read_content = if let Some((start, end)) = range {
        read_line_range_bounded(path, start, end)
    } else {
        read_text_prefix_bounded(path)
    }
    .with_context(|| format!("failed to read text file {}", path.display()))?;

    Ok(ObservedRead {
        data: read_data(workspace.relative(path), line_label, read_content.content),
        content_truncated: read_content.truncated,
    })
}

fn observed_read_args(workspace: &Workspace, args: &ReadArgs) -> Result<ObservedRead> {
    let path = workspace.resolve_existing_workspace_path(&args.path)?;
    let range = args
        .lines
        .as_deref()
        .map(parse_line_range)
        .transpose()
        .context("invalid --lines range")?;
    observed_read(workspace, &path, range)
}

fn read_observation(read: ObservedRead) -> Observation<ReadData> {
    let data = read.data;
    let content_truncated = read.content_truncated;
    let summary = read_summary(&data.path, data.lines.as_deref(), content_truncated);
    let evidence = read_evidence(&data);
    let next_observations = read_followup_observations(&data.path);
    observation_with_evidence(
        WORKSPACE_READ_KIND,
        data.path.clone(),
        summary,
        data,
        evidence,
        content_truncated,
        next_observations,
    )
}

fn read_followup_observations(path: &str) -> Vec<String> {
    vec![
        format!("workspace search {}", shell_hint(path)),
        WORKSPACE_DIFF_SUMMARY_COMMAND.to_string(),
    ]
}

fn observed_diff(workspace: &Workspace, summary_only: bool) -> Result<ObservedDiff> {
    if workspace.is_git_repo {
        git_observed_diff(workspace, summary_only)
    } else {
        Ok(non_repo_observed_diff())
    }
}

fn git_observed_diff(workspace: &Workspace, summary_only: bool) -> Result<ObservedDiff> {
    let summary_output =
        git_observable_diff_output_bounded(workspace, ["--stat"], MAX_DIFF_SUMMARY)?;
    let diff_files = git_observable_diff_name_only(workspace, MAX_CHANGED_FILES)?;
    let (patch, patch_truncated) = if summary_only {
        (None, false)
    } else {
        let patch = git_observable_diff_output_bounded(workspace, [], MAX_DIFF_PATCH)?;
        (Some(patch.text), patch.truncated)
    };
    Ok(ObservedDiff {
        data: diff_data(true, summary_output.text, diff_files, patch),
        summary_truncated: summary_output.truncated,
        patch_truncated,
    })
}

fn non_repo_observed_diff() -> ObservedDiff {
    ObservedDiff {
        data: diff_data(
            false,
            SUMMARY_NOT_GIT_REPOSITORY.to_string(),
            BoundedFileList::empty(),
            None,
        ),
        summary_truncated: false,
        patch_truncated: false,
    }
}

fn diff_data(
    is_repo: bool,
    summary: String,
    changed_files: BoundedFileList,
    patch: Option<String>,
) -> DiffData {
    DiffData {
        is_repo,
        summary,
        file_count: changed_files.total_files,
        files: changed_files.files,
        omitted_files: changed_files.omitted_files,
        patch,
    }
}

fn diff_observation(workspace: &Workspace, diff: ObservedDiff) -> Observation<DiffData> {
    let data = diff.data;
    let summary = diff_summary(&data, diff.summary_truncated, diff.patch_truncated);
    let evidence = changed_file_evidence(&data.files, EVIDENCE_REASON_GIT_DIFF_CHANGED_FILE);
    let truncated = diff_truncated(&data, diff.summary_truncated, diff.patch_truncated);
    let next_observations =
        read_next_observations(workspace, data.files.iter().map(String::as_str));
    observation_with_evidence(
        WORKSPACE_DIFF_KIND,
        workspace.root.to_string_lossy().into_owned(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    )
}

fn observed_map(workspace: &Workspace, args: &MapArgs) -> Result<WorkspaceMap> {
    build_map(workspace, args.depth, args.include_hidden)
}

fn map_observation(map: WorkspaceMap) -> Observation<WorkspaceMap> {
    let truncated = map_truncated(&map);
    let summary = map_summary(&map, truncated);
    let evidence = map_evidence(&map);
    let next_observations = map_next_observations(&map);
    observation_with_evidence(
        WORKSPACE_MAP_KIND,
        map.root.clone(),
        summary,
        map,
        evidence,
        truncated,
        next_observations,
    )
}

fn read_status_recent_operations(workspace: &Workspace, limit: usize) -> StatusRecentOperations {
    match read_log(workspace, limit) {
        Ok(window) => StatusRecentOperations {
            entries: window.entries,
            omitted_lines: window.omitted_lines,
            error: None,
        },
        Err(error) => StatusRecentOperations {
            entries: vec![],
            omitted_lines: 0,
            error: Some(format!("{error:#}")),
        },
    }
}

fn observed_status(workspace: &Workspace) -> Result<StatusData> {
    let git = git_summary(workspace)?;
    let index_status = cochange_index_status(workspace);
    let recent_operations = read_status_recent_operations(workspace, 10);
    Ok(status_data(workspace, git, index_status, recent_operations))
}

fn status_data(
    workspace: &Workspace,
    git: GitSummary,
    index_status: IndexStatusData,
    recent_operations: StatusRecentOperations,
) -> StatusData {
    StatusData {
        root: workspace.root.to_string_lossy().into_owned(),
        git,
        index_status,
        recent_operations: recent_operations.entries,
        recent_operations_omitted: recent_operations.omitted_lines,
        recent_operations_error: recent_operations.error,
    }
}

fn status_observation(data: StatusData) -> Observation<StatusData> {
    let truncated = status_truncated(&data);
    let summary = status_summary(&data, truncated);
    observation_without_evidence(
        WORKSPACE_STATUS_KIND,
        data.root.clone(),
        summary,
        data,
        truncated,
        status_next_observations(),
    )
}

fn observed_index_status(workspace: &Workspace) -> IndexStatusData {
    cochange_index_status(workspace)
}

fn index_status_observation(data: IndexStatusData) -> Observation<IndexStatusData> {
    let summary = index_status_summary(&data);
    observation_without_evidence(
        WORKSPACE_INDEX_STATUS_KIND,
        data.path.clone(),
        summary,
        data,
        false,
        index_status_next_observations(),
    )
}

fn index_cochange_data(
    workspace: &Workspace,
    index_path: &Path,
    index: &CochangeIndex,
) -> IndexCochangeData {
    IndexCochangeData {
        path: workspace.relative(index_path),
        version: index.version,
        generated_at_unix_ms: index.generated_at_unix_ms,
        head: index.head.clone(),
        max_commits: index.max_commits,
        max_files_per_commit: index.max_files_per_commit,
        commits_scanned: index.commits_scanned,
        commits_indexed: index.commits_indexed,
        ignored_large_commits: index.ignored_large_commits,
        file_count: index.file_commit_counts.len(),
        edge_count: index.edges.len(),
    }
}

fn observed_index_cochange(
    workspace: &Workspace,
    args: &IndexCochangeArgs,
) -> Result<IndexCochangeData> {
    if !workspace.is_git_repo {
        bail!("workspace index cochange requires a git repository");
    }

    ensure_log_writable(workspace)?;
    let index = build_cochange_index(workspace, args.max_commits, args.max_files_per_commit)?;
    let index_path = write_workspace_cochange_index(workspace, &index)?;
    Ok(index_cochange_data(workspace, &index_path, &index))
}

fn write_workspace_cochange_index(workspace: &Workspace, index: &CochangeIndex) -> Result<PathBuf> {
    let index_path = workspace.cochange_index_path();
    let index_dir = workspace.root.join(INDEX_DIR);
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("failed to create index directory {}", index_dir.display()))?;
    write_cochange_index(&index_path, index)
        .with_context(|| format!("failed to write index {}", index_path.display()))?;
    Ok(index_path)
}

fn index_cochange_observation(data: IndexCochangeData) -> Observation<IndexCochangeData> {
    let summary = index_cochange_summary(&data);
    observation_without_evidence(
        WORKSPACE_INDEX_COCHANGE_KIND,
        data.path.clone(),
        summary,
        data,
        false,
        index_cochange_next_observations(),
    )
}

fn observed_related(
    workspace: &Workspace,
    target: &str,
    args: &RelatedArgs,
) -> Result<RelatedData> {
    if workspace.is_git_repo {
        related_by_cochange(
            workspace,
            target,
            args.max_commits,
            args.max_files_per_commit,
            args.max_results,
            args.rank,
            args.use_index,
        )
    } else {
        Ok(related_data_for_non_repo(
            target,
            &args.by,
            args.rank,
            args.use_index,
            args.max_commits,
            args.max_files_per_commit,
        ))
    }
}

fn observed_related_args(workspace: &Workspace, args: &RelatedArgs) -> Result<ObservedRelated> {
    let target = workspace_arg_path(workspace, &args.path)?;
    let data = observed_related(workspace, &target, args)?;
    Ok(ObservedRelated { target, data })
}

fn related_observation(
    workspace: &Workspace,
    target: &str,
    data: RelatedData,
) -> Observation<RelatedData> {
    let summary = related_summary(&data);
    let evidence = related_evidence(&data);
    let next_observations = read_next_observations(
        workspace,
        data.related.iter().map(|file| file.path.as_str()),
    );
    observation_with_evidence(
        WORKSPACE_RELATED_KIND,
        target.to_string(),
        summary,
        data,
        evidence,
        false,
        next_observations,
    )
}

fn observed_impact_args(workspace: &Workspace, args: &ImpactArgs) -> Result<ImpactData> {
    if !args.diff {
        bail!("workspace impact currently supports only --diff as its source");
    }

    observed_impact(workspace, args)
}

fn observed_impact(workspace: &Workspace, args: &ImpactArgs) -> Result<ImpactData> {
    if workspace.is_git_repo {
        impact_by_cochange(
            workspace,
            args.max_commits,
            args.max_files_per_commit,
            args.max_results,
            args.rank,
            args.use_index,
        )
    } else {
        Ok(impact_data_for_non_repo(
            &args.by,
            args.rank,
            args.use_index,
            args.max_commits,
            args.max_files_per_commit,
        ))
    }
}

fn impact_observation(workspace: &Workspace, data: ImpactData) -> Observation<ImpactData> {
    let summary = impact_summary(&data);
    let evidence = impact_evidence(&data);
    let truncated = impact_truncated(&data);
    let next_observations = read_next_observations(
        workspace,
        data.impacted.iter().map(|file| file.path.as_str()),
    );
    observation_with_evidence(
        WORKSPACE_IMPACT_KIND,
        data.source.clone(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    )
}

fn observed_run(
    workspace: &Workspace,
    command: &str,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
) -> ObservedRun {
    let output_truncated = captured_outputs_truncated(&stdout, &stderr);
    ObservedRun {
        data: run_data(
            command,
            workspace.root.to_string_lossy().into_owned(),
            exit_code,
            duration_ms,
            stdout.text,
            stderr.text,
        ),
        output_truncated,
    }
}

fn captured_outputs_truncated(stdout: &CapturedOutput, stderr: &CapturedOutput) -> bool {
    stdout.truncated || stderr.truncated
}

fn run_data(
    command: &str,
    cwd: String,
    exit_code: Option<i32>,
    duration_ms: u128,
    stdout: String,
    stderr: String,
) -> RunData {
    RunData {
        command: command.to_string(),
        cwd,
        exit_code,
        duration_ms,
        stdout,
        stderr,
    }
}

fn run_observation(run: ObservedRun) -> Observation<RunData> {
    let data = run.data;
    let summary = run_summary(data.exit_code, data.duration_ms, run.output_truncated);
    observation_without_evidence(
        WORKSPACE_RUN_KIND,
        data.command.clone(),
        summary,
        data,
        run.output_truncated,
        run_followup_observations(),
    )
}

fn log_data(workspace: &Workspace, window: LogWindow) -> LogData {
    LogData {
        log_path: workspace.relative(&workspace.log_path()),
        omitted_lines: window.omitted_lines,
        entries: window.entries,
    }
}

fn observed_log(workspace: &Workspace, args: &LogArgs) -> Result<LogData> {
    let window = read_log(workspace, args.limit)?;
    Ok(log_data(workspace, window))
}

fn log_observation(data: LogData) -> Observation<LogData> {
    let summary = log_summary(&data);
    let truncated = log_truncated(&data);
    observation_without_evidence(
        WORKSPACE_LOG_KIND,
        data.log_path.clone(),
        summary,
        data,
        truncated,
        log_followup_observations(),
    )
}

fn observation_without_evidence<T: Serialize>(
    kind: &str,
    scope: String,
    summary: String,
    data: T,
    truncated: bool,
    next_observations: Vec<String>,
) -> Observation<T> {
    observation_with_evidence(
        kind,
        scope,
        summary,
        data,
        vec![],
        truncated,
        next_observations,
    )
}

fn observation_with_evidence<T: Serialize>(
    kind: &str,
    scope: String,
    summary: String,
    data: T,
    evidence: Vec<Evidence>,
    truncated: bool,
    next_observations: Vec<String>,
) -> Observation<T> {
    Observation {
        kind: kind.to_string(),
        scope,
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    }
}

fn static_observation_commands(commands: &[&str]) -> Vec<String> {
    commands
        .iter()
        .map(|command| (*command).to_string())
        .collect()
}

fn status_next_observations() -> Vec<String> {
    static_observation_commands(&[
        WORKSPACE_MAP_COMMAND,
        WORKSPACE_DIFF_SUMMARY_COMMAND,
        WORKSPACE_INDEX_STATUS_COMMAND,
        WORKSPACE_LOG_COMMAND,
    ])
}

fn index_status_next_observations() -> Vec<String> {
    let mut next = static_observation_commands(&[WORKSPACE_INDEX_COCHANGE_COMMAND]);
    next.extend(index_cochange_next_observations());
    next
}

fn index_cochange_next_observations() -> Vec<String> {
    static_observation_commands(&[
        WORKSPACE_RELATED_INDEX_COMMAND,
        WORKSPACE_IMPACT_INDEX_COMMAND,
    ])
}

fn related_data_for_non_repo(
    target: &str,
    method: &RelatedMethod,
    rank: RankingMethod,
    use_index: bool,
    max_commits: usize,
    max_files_per_commit: usize,
) -> RelatedData {
    cochange_related_data(
        RelatedDataMetadata::new(
            target,
            method,
            rank,
            relationship_source_for_options(use_index, rank),
            false,
        ),
        RelationshipStats::none(),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        vec![],
    )
}

fn cochange_related_data(
    metadata: RelatedDataMetadata,
    stats: RelationshipStats,
    limits: RelationshipLimits,
    related: Vec<RelatedFile>,
) -> RelatedData {
    let RelatedDataMetadata {
        target,
        relationship,
    } = metadata;
    let (method, ranking, relationship_source, is_repo) = relationship.into_parts();
    let (commits_scanned, commits_matched, ignored_large_commits) = stats.into_parts();
    let (max_commits, max_files_per_commit) = limits.into_parts();

    RelatedData {
        target,
        method,
        ranking,
        relationship_source,
        is_repo,
        commits_scanned,
        commits_matched,
        ignored_large_commits,
        max_commits,
        max_files_per_commit,
        related,
    }
}

fn impact_data_for_non_repo(
    method: &RelatedMethod,
    rank: RankingMethod,
    use_index: bool,
    max_commits: usize,
    max_files_per_commit: usize,
) -> ImpactData {
    cochange_impact_data(
        ImpactDataMetadata::new(
            method,
            rank,
            relationship_source_for_options(use_index, rank),
            false,
        ),
        SeedFileSummary::empty(),
        RelationshipStats::none(),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        vec![],
    )
}

fn cochange_impact_data(
    metadata: ImpactDataMetadata,
    seed_summary: SeedFileSummary,
    stats: RelationshipStats,
    limits: RelationshipLimits,
    impacted: Vec<ImpactFile>,
) -> ImpactData {
    let ImpactDataMetadata {
        source,
        relationship,
    } = metadata;
    let (method, ranking, relationship_source, is_repo) = relationship.into_parts();
    let (commits_scanned, commits_matched, ignored_large_commits) = stats.into_parts();
    let (max_commits, max_files_per_commit) = limits.into_parts();

    ImpactData {
        source,
        method,
        ranking,
        relationship_source,
        is_repo,
        seed_files: seed_summary.seed_files,
        seed_file_count: seed_summary.seed_file_count,
        omitted_seed_files: seed_summary.omitted_seed_files,
        commits_scanned,
        commits_matched,
        ignored_large_commits,
        max_commits,
        max_files_per_commit,
        impacted,
    }
}

fn patch_followup_observations(transaction_id: &str) -> Vec<String> {
    vec![
        WORKSPACE_DIFF_SUMMARY_COMMAND.to_string(),
        format!("workspace rollback {transaction_id}"),
    ]
}

fn run_followup_observations() -> Vec<String> {
    static_observation_commands(&[WORKSPACE_STATUS_COMMAND, WORKSPACE_DIFF_SUMMARY_COMMAND])
}

fn log_followup_observations() -> Vec<String> {
    static_observation_commands(&[WORKSPACE_STATUS_COMMAND])
}

fn rollback_followup_observations() -> Vec<String> {
    static_observation_commands(&[WORKSPACE_DIFF_SUMMARY_COMMAND])
}

fn map_truncated(map: &WorkspaceMap) -> bool {
    map.omitted.any() || map.git.omitted_files()
}

fn map_summary(map: &WorkspaceMap, truncated: bool) -> String {
    let mut summary = map_file_language_summary(map);
    append_note_if(&mut summary, truncated, SUMMARY_NOTE_MAP_TRUNCATED);
    summary
}

fn map_file_language_summary(map: &WorkspaceMap) -> String {
    format!(
        "{} file(s), languages: {}",
        map.stats.file_count,
        join_or_none(&map.stack.languages)
    )
}

fn status_truncated(data: &StatusData) -> bool {
    data.git.omitted_files() || status_recent_operations_omitted(data)
}

fn status_summary(data: &StatusData, truncated: bool) -> String {
    let mut summary = if data.git.is_repo {
        status_repository_summary(data)
    } else {
        SUMMARY_NOT_GIT_REPOSITORY.to_string()
    };
    append_note_if(&mut summary, truncated, SUMMARY_NOTE_STATUS_TRUNCATED);
    summary
}

fn status_repository_summary(data: &StatusData) -> String {
    format!(
        "branch {}, {} dirty file(s), {} untracked file(s), index {}{}",
        data.git.branch.as_deref().unwrap_or("unknown"),
        data.git.dirty_file_count,
        data.git.untracked_file_count,
        data.index_status.status,
        status_log_note(data)
    )
}

fn status_log_note(data: &StatusData) -> &'static str {
    if data.recent_operations_error.is_some() {
        SUMMARY_NOTE_OPERATION_LOG_UNREADABLE
    } else if status_recent_operations_omitted(data) {
        SUMMARY_NOTE_RECENT_OPERATIONS_TRUNCATED
    } else {
        ""
    }
}

fn status_recent_operations_omitted(data: &StatusData) -> bool {
    data.recent_operations_omitted > 0
}

fn index_status_summary(data: &IndexStatusData) -> String {
    index_status_summary_label(&data.status)
        .unwrap_or(data.status.as_str())
        .to_string()
}

fn index_status_summary_label(status: &str) -> Option<&'static str> {
    match status {
        INDEX_STATUS_FRESH => Some("co-change index is fresh"),
        INDEX_STATUS_STALE => Some("co-change index is stale"),
        INDEX_STATUS_MISSING => Some("co-change index is missing"),
        INDEX_STATUS_INVALID => Some("co-change index is invalid"),
        INDEX_STATUS_NOT_GIT_REPO => Some(SUMMARY_NOT_GIT_REPOSITORY),
        _ => None,
    }
}

fn index_cochange_summary(data: &IndexCochangeData) -> String {
    format!(
        "indexed {} co-change edge(s) from {} commit(s)",
        data.edge_count, data.commits_indexed
    )
}

fn related_summary(data: &RelatedData) -> String {
    if data.is_repo {
        related_repository_summary(data)
    } else {
        SUMMARY_NOT_GIT_REPOSITORY.to_string()
    }
}

fn related_repository_summary(data: &RelatedData) -> String {
    format!(
        "{} related file(s) for {} using {} history",
        data.related.len(),
        data.target,
        data.method
    )
}

fn impact_summary(data: &ImpactData) -> String {
    if data.is_repo {
        let mut summary = impact_repository_summary(data);
        append_note_if(
            &mut summary,
            impact_seed_files_omitted(data),
            SUMMARY_NOTE_SEED_FILES_TRUNCATED,
        );
        summary
    } else {
        SUMMARY_NOT_GIT_REPOSITORY.to_string()
    }
}

fn impact_repository_summary(data: &ImpactData) -> String {
    format!(
        "{} impacted file(s) from {} seed file(s) using {} history",
        data.impacted.len(),
        data.seed_file_count,
        data.method
    )
}

fn search_truncated(data: &SearchData) -> bool {
    search_results_omitted(data) || search_match_texts_truncated(data)
}

fn search_results_omitted(data: &SearchData) -> bool {
    data.total_matches > data.matches.len()
}

fn search_match_texts_truncated(data: &SearchData) -> bool {
    data.truncated_match_texts > 0
}

fn search_summary(data: &SearchData) -> String {
    let mut summary = search_result_count_summary(data);
    append_search_match_text_truncation_note(&mut summary, data);
    summary
}

fn search_result_count_summary(data: &SearchData) -> String {
    if search_results_omitted(data) {
        format!(
            "{} match(es) for {:?}, showing {}",
            data.total_matches,
            data.query,
            data.matches.len()
        )
    } else {
        format!("{} match(es) for {:?}", data.total_matches, data.query)
    }
}

fn append_search_match_text_truncation_note(summary: &mut String, data: &SearchData) {
    if search_match_texts_truncated(data) {
        summary.push_str(&format!(
            ", truncated {} match text(s)",
            data.truncated_match_texts
        ));
    }
}

fn impact_truncated(data: &ImpactData) -> bool {
    impact_seed_files_omitted(data)
}

fn impact_seed_files_omitted(data: &ImpactData) -> bool {
    data.omitted_seed_files > 0
}

fn diff_truncated(data: &DiffData, summary_truncated: bool, patch_truncated: bool) -> bool {
    summary_truncated || patch_truncated || diff_files_omitted(data)
}

fn diff_summary(data: &DiffData, summary_truncated: bool, patch_truncated: bool) -> String {
    let mut summary = if data.is_repo {
        diff_repository_summary(data)
    } else {
        data.summary.clone()
    };
    if let Some(note) = diff_output_truncation_note(summary_truncated, patch_truncated) {
        summary.push_str(note);
    }
    append_note_if(
        &mut summary,
        diff_files_omitted(data),
        SUMMARY_NOTE_FILES_TRUNCATED,
    );
    summary
}

fn diff_repository_summary(data: &DiffData) -> String {
    format!("{} changed file(s)", data.file_count)
}

fn diff_files_omitted(data: &DiffData) -> bool {
    data.omitted_files > 0
}

fn diff_output_truncation_note(
    summary_truncated: bool,
    patch_truncated: bool,
) -> Option<&'static str> {
    match (summary_truncated, patch_truncated) {
        (true, true) => Some(SUMMARY_NOTE_SUMMARY_AND_PATCH_TRUNCATED),
        (true, false) => Some(SUMMARY_NOTE_SUMMARY_TRUNCATED),
        (false, true) => Some(SUMMARY_NOTE_PATCH_TRUNCATED),
        (false, false) => None,
    }
}

fn transaction_file_summary(
    action: &str,
    transaction_id: &str,
    file_count: usize,
    omitted_files: usize,
) -> String {
    let mut summary =
        format!("{action} transaction {transaction_id} touching {file_count} file(s)");
    append_note_if(
        &mut summary,
        transaction_files_truncated(omitted_files),
        SUMMARY_NOTE_FILES_TRUNCATED,
    );
    summary
}

fn run_summary(exit_code: Option<i32>, duration_ms: u128, truncated: bool) -> String {
    let status = run_exit_status_label(exit_code);
    let mut summary = format!("command exited with {status} in {duration_ms}ms");
    append_note_if(&mut summary, truncated, SUMMARY_NOTE_OUTPUT_TRUNCATED);
    summary
}

fn run_exit_status_label(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

fn read_summary(path: &str, lines: Option<&str>, truncated: bool) -> String {
    let mut summary = read_target_summary(path, lines);
    append_note_if(&mut summary, truncated, SUMMARY_NOTE_READ_TRUNCATED);
    summary
}

fn read_target_summary(path: &str, lines: Option<&str>) -> String {
    match lines {
        Some(lines) => format!("read {path} lines {lines}"),
        None => format!("read {path}"),
    }
}

fn log_summary(data: &LogData) -> String {
    let mut summary = log_entry_count_summary(data);
    append_log_lines_omission_note(&mut summary, data);
    summary
}

fn log_entry_count_summary(data: &LogData) -> String {
    format!("{} operation(s)", data.entries.len())
}

fn append_log_lines_omission_note(summary: &mut String, data: &LogData) {
    if log_lines_omitted(data) {
        summary.push_str(&format!(
            " ({} older log line(s) omitted)",
            data.omitted_lines
        ));
    }
}

fn log_truncated(data: &LogData) -> bool {
    log_lines_omitted(data)
}

fn log_lines_omitted(data: &LogData) -> bool {
    data.omitted_lines > 0
}

fn append_note_if(summary: &mut String, condition: bool, note: &str) {
    if condition {
        summary.push_str(note);
    }
}

fn transaction_patch_path(workspace: &Workspace, transaction_id: &str) -> Result<PathBuf> {
    validate_patch_transaction_id(transaction_id)?;
    Ok(workspace
        .transaction_dir()
        .join(format!("{transaction_id}.patch")))
}

fn validate_patch_transaction_id(transaction_id: &str) -> Result<()> {
    let Some(rest) = transaction_id.strip_prefix("tx-") else {
        bail!("invalid transaction id {transaction_id:?}; expected tx-<digits>");
    };
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid transaction id {transaction_id:?}; expected tx-<digits>");
    }
    Ok(())
}

fn build_map(workspace: &Workspace, depth: usize, include_hidden: bool) -> Result<WorkspaceMap> {
    let git = git_summary(workspace)?;
    let mut signals = MapSignals::default();
    let mut file_count = 0usize;
    let mut directory_count = 0usize;
    let mut large_file_count = 0usize;
    let mut large_files = Vec::new();
    let mut recent_candidates = Vec::new();

    for entry in WalkDir::new(&workspace.root)
        .max_depth(depth)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.path() == workspace.root || should_descend(entry.path(), include_hidden)
        })
    {
        let entry = entry?;
        let path = entry.path();
        if path == workspace.root {
            continue;
        }
        let rel = workspace.relative(path);
        if entry.file_type().is_dir() {
            directory_count += 1;
            signals.directories.push(rel);
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        file_count += 1;
        let metadata = entry.metadata()?;
        if metadata.len() > 1_000_000 {
            large_file_count += 1;
            push_large_file_candidate(
                &mut large_files,
                LargeFile {
                    path: rel.clone(),
                    bytes: metadata.len(),
                },
            );
        }
        if let Ok(modified) = metadata.modified() {
            push_recent_candidate(&mut recent_candidates, modified, rel.clone());
        }
        record_map_file(&mut signals, &rel);
    }

    sort_recent_candidates(&mut recent_candidates);
    let recent_files = recent_candidates
        .into_iter()
        .take(MAX_RECENT_FILES)
        .map(|(_, path)| path)
        .collect::<Vec<_>>();

    let stack = detect_stack(workspace, &signals)?;
    let (structure, mut omitted) = detect_structure(&signals);
    let commands = detect_commands(workspace, &signals)?;
    let important_files = important_files(&structure, &stack);
    sort_large_files(&mut large_files);
    omitted.large_files = large_file_count.saturating_sub(large_files.len());

    Ok(WorkspaceMap {
        root: workspace.root.to_string_lossy().into_owned(),
        git,
        stack,
        structure,
        commands,
        stats: WorkspaceStats {
            file_count,
            directory_count,
            large_files,
        },
        important_files,
        recent_files,
        omitted,
    })
}

fn truncate_vec<T>(items: &mut Vec<T>, max_len: usize) -> usize {
    let omitted = items.len().saturating_sub(max_len);
    if omitted > 0 {
        items.truncate(max_len);
    }
    omitted
}

fn observed_string_prefix(items: &[String], max_len: usize) -> (Vec<String>, usize) {
    let observed = items.iter().take(max_len).cloned().collect::<Vec<_>>();
    let omitted = items.len().saturating_sub(observed.len());
    (observed, omitted)
}

fn push_bounded_sorted<T>(
    items: &mut Vec<T>,
    item: T,
    max_len: usize,
    compare: fn(&T, &T) -> std::cmp::Ordering,
) {
    if max_len == 0 {
        return;
    }
    let index = items
        .binary_search_by(|existing| compare(existing, &item))
        .unwrap_or_else(|index| index);
    if index >= max_len {
        return;
    }
    items.insert(index, item);
    if items.len() > max_len {
        items.pop();
    }
}

fn sort_and_truncate<T>(
    items: &mut Vec<T>,
    max_len: usize,
    compare: fn(&T, &T) -> std::cmp::Ordering,
) {
    items.sort_by(compare);
    items.truncate(max_len);
}

fn push_recent_candidate(
    recent_candidates: &mut Vec<(SystemTime, String)>,
    modified: SystemTime,
    path: String,
) {
    push_bounded_sorted(
        recent_candidates,
        (modified, path),
        MAX_RECENT_FILES,
        compare_recent_candidate,
    );
}

fn sort_recent_candidates(recent_candidates: &mut [(SystemTime, String)]) {
    recent_candidates.sort_by(compare_recent_candidate);
}

fn compare_recent_candidate(
    a: &(SystemTime, String),
    b: &(SystemTime, String),
) -> std::cmp::Ordering {
    b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1))
}

fn push_large_file_candidate(large_files: &mut Vec<LargeFile>, item: LargeFile) {
    push_bounded_sorted(
        large_files,
        item,
        MAX_MAP_LARGE_FILES,
        compare_large_file_by_size,
    );
}

fn sort_large_files(large_files: &mut [LargeFile]) {
    large_files.sort_by(compare_large_file_by_size);
}

fn compare_large_file_by_size(a: &LargeFile, b: &LargeFile) -> std::cmp::Ordering {
    b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path))
}

fn should_descend(path: &Path, include_hidden: bool) -> bool {
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or("");
    if matches!(
        name,
        ".git" | LOG_DIR | "target" | "node_modules" | ".next" | "dist" | "build"
    ) {
        return false;
    }
    include_hidden || !name.starts_with('.')
}

fn record_map_file(signals: &mut MapSignals, path: &str) {
    match Path::new(path).extension().and_then(OsStr::to_str) {
        Some("rs") => {
            signals.languages.insert("rust".to_string());
        }
        Some("ts") | Some("tsx") => {
            signals.languages.insert("typescript".to_string());
        }
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => {
            signals.languages.insert("javascript".to_string());
        }
        Some("py") => {
            signals.languages.insert("python".to_string());
        }
        Some("go") => {
            signals.languages.insert("go".to_string());
        }
        Some("java") => {
            signals.languages.insert("java".to_string());
        }
        Some("md") => {
            signals.languages.insert("markdown".to_string());
        }
        _ => {}
    }

    if is_named_map_file(path) {
        signals.named_files.insert(path.to_string());
    }
    if is_test_file(path) {
        signals.tests.push(path.to_string());
    }
    if path.ends_with(".config.js") && !MAP_CONFIG_NAMES.contains(&path) {
        signals.config_extras.push(path.to_string());
    }

    let lower = path.to_lowercase();
    if lower == "readme.md" || lower.starts_with("docs/") || lower.ends_with(".md") {
        signals.docs.push(path.to_string());
    }
}

fn is_named_map_file(path: &str) -> bool {
    MAP_ENTRYPOINT_NAMES.contains(&path)
        || MAP_CONFIG_NAMES.contains(&path)
        || MAP_STACK_ONLY_NAMES.contains(&path)
}

fn detect_stack(workspace: &Workspace, signals: &MapSignals) -> Result<StackSummary> {
    let mut package_managers = BTreeSet::new();
    let mut frameworks = BTreeSet::new();

    if signals.named_files.contains("Cargo.toml") {
        package_managers.insert("cargo".to_string());
        frameworks.insert("rust-cli".to_string());
    }
    if signals.named_files.contains("package.json") {
        package_managers.insert("npm".to_string());
        let package_json = workspace.root.join("package.json");
        if let Ok(detected_frameworks) = detect_package_json_frameworks(&package_json) {
            for framework in detected_frameworks {
                frameworks.insert(framework);
            }
        }
    }
    if signals.named_files.contains("pnpm-lock.yaml") {
        package_managers.insert("pnpm".to_string());
    }
    if signals.named_files.contains("yarn.lock") {
        package_managers.insert("yarn".to_string());
    }
    if signals.named_files.contains("go.mod") {
        package_managers.insert("go".to_string());
    }
    if signals.named_files.contains("pyproject.toml") {
        package_managers.insert("python/pyproject".to_string());
    }
    if signals.named_files.contains("requirements.txt") {
        package_managers.insert("pip".to_string());
    }

    Ok(StackSummary {
        languages: signals.languages.iter().cloned().collect(),
        package_managers: package_managers.into_iter().collect(),
        frameworks: frameworks.into_iter().collect(),
    })
}

fn detect_structure(signals: &MapSignals) -> (StructureSummary, MapOmittedCounts) {
    let entrypoints = MAP_ENTRYPOINT_NAMES
        .iter()
        .filter(|path| signals.named_files.contains(**path))
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    let mut configs = MAP_CONFIG_NAMES
        .iter()
        .filter(|path| signals.named_files.contains(**path))
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    let config_count = configs.len() + signals.config_extras.total_items();
    let config_extras = signals.config_extras.observed();
    configs.extend(config_extras);
    truncate_vec(&mut configs, MAX_MAP_LIST_ITEMS);

    let structure = StructureSummary {
        directories: signals.directories.observed(),
        entrypoints,
        tests: signals.tests.observed(),
        configs,
        docs: signals.docs.observed(),
    };
    let omitted = MapOmittedCounts {
        directories: signals.directories.omitted_count(),
        entrypoints: 0,
        tests: signals.tests.omitted_count(),
        configs: config_count.saturating_sub(structure.configs.len()),
        docs: signals.docs.omitted_count(),
        large_files: 0,
    };

    (structure, omitted)
}

fn detect_package_json_frameworks(path: &Path) -> Result<Vec<String>> {
    let needles = [
        (b"\"next\"".as_slice(), "nextjs"),
        (b"\"react\"".as_slice(), "react"),
        (b"\"vue\"".as_slice(), "vue"),
        (b"\"svelte\"".as_slice(), "svelte"),
        (b"\"vite\"".as_slice(), "vite"),
        (b"\"express\"".as_slice(), "express"),
    ];
    let matched = file_contains_needles(path, &needles)?;
    Ok(needles
        .iter()
        .zip(matched)
        .filter(|(_, matched)| *matched)
        .map(|((_, name), _)| (*name).to_string())
        .collect())
}

fn file_contains_needles(path: &Path, needles: &[(&[u8], &str)]) -> Result<Vec<bool>> {
    let mut file = fs::File::open(path)?;
    let mut matched = vec![false; needles.len()];
    let max_needle_len = needles
        .iter()
        .map(|(needle, _)| needle.len())
        .max()
        .unwrap_or(0);
    let mut tail = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        for (index, (needle, _)) in needles.iter().enumerate() {
            if !matched[index]
                && find_query_with_tail(&tail, &buffer[..bytes_read], needle).is_some()
            {
                matched[index] = true;
            }
        }
        if matched.iter().all(|matched| *matched) {
            break;
        }
        replace_scan_tail(
            &mut tail,
            &buffer[..bytes_read],
            max_needle_len.saturating_sub(1),
        );
    }

    Ok(matched)
}

fn read_json_file_bounded(path: &Path, max_bytes: u64) -> Result<Value> {
    let file = fs::File::open(path)?;
    let bytes = file.metadata()?.len();
    if bytes > max_bytes {
        bail!("JSON file {} exceeded {} bytes", path.display(), max_bytes);
    }
    serde_json::from_reader(BufReader::new(file)).context("failed to parse JSON")
}

fn detect_commands(
    workspace: &Workspace,
    signals: &MapSignals,
) -> Result<BTreeMap<String, String>> {
    let mut commands = BTreeMap::new();

    if signals.named_files.contains("Cargo.toml") {
        commands.insert("build".to_string(), "cargo build".to_string());
        commands.insert("test".to_string(), "cargo test".to_string());
        commands.insert("run".to_string(), "cargo run --".to_string());
    }

    if signals.named_files.contains("package.json") {
        let package_json = workspace.root.join("package.json");
        if let Ok(value) = read_json_file_bounded(&package_json, MAX_PACKAGE_JSON_BYTES)
            && let Some(scripts) = value.get("scripts").and_then(Value::as_object)
        {
            for (name, value) in scripts {
                if let Some(script) = value.as_str() {
                    commands.insert(name.clone(), format!("npm run {name} # {script}"));
                }
            }
        }
    }

    if signals.named_files.contains("Makefile") {
        commands
            .entry("make".to_string())
            .or_insert("make".to_string());
    }
    if signals.named_files.contains("justfile") {
        commands
            .entry("just".to_string())
            .or_insert("just".to_string());
    }

    Ok(commands)
}

fn important_files(structure: &StructureSummary, stack: &StackSummary) -> Vec<ImportantFile> {
    let mut items = Vec::new();
    for path in &structure.configs {
        items.push(ImportantFile {
            path: path.clone(),
            reason: IMPORTANT_REASON_CONFIGURATION_OR_PACKAGE_MANIFEST.to_string(),
        });
    }
    for path in &structure.entrypoints {
        items.push(ImportantFile {
            path: path.clone(),
            reason: IMPORTANT_REASON_LIKELY_ENTRYPOINT.to_string(),
        });
    }
    if let Some(doc) = structure
        .docs
        .iter()
        .find(|path| path.eq_ignore_ascii_case("README.md"))
    {
        items.push(ImportantFile {
            path: doc.clone(),
            reason: IMPORTANT_REASON_PRIMARY_PROJECT_DOCUMENTATION.to_string(),
        });
    }
    if stack.languages.is_empty() {
        items.push(ImportantFile {
            path: ".".to_string(),
            reason: IMPORTANT_REASON_NO_LANGUAGE_SIGNALS.to_string(),
        });
    }
    items
}

fn map_evidence(map: &WorkspaceMap) -> Vec<Evidence> {
    map.important_files
        .iter()
        .take(MAX_MAP_EVIDENCE_ITEMS)
        .map(|file| Evidence {
            path: file.path.clone(),
            lines: None,
            reason: file.reason.clone(),
        })
        .collect()
}

fn map_next_observations(map: &WorkspaceMap) -> Vec<String> {
    let mut next = Vec::new();
    if map.structure.docs.iter().any(|path| path == "README.md") {
        next.push(workspace_read_command("README.md"));
    }
    for file in map
        .important_files
        .iter()
        .take(MAX_MAP_IMPORTANT_NEXT_OBSERVATIONS)
    {
        if file.path != "README.md" && file.path != "." {
            next.push(workspace_read_command(&file.path));
        }
    }
    if map.git.is_repo {
        next.push(WORKSPACE_DIFF_SUMMARY_COMMAND.to_string());
        next.push(WORKSPACE_INDEX_STATUS_COMMAND.to_string());
        next.push(WORKSPACE_INDEX_COCHANGE_COMMAND.to_string());
        next.push(WORKSPACE_IMPACT_COCHANGE_COMMAND.to_string());
        if let Some(entrypoint) = map.structure.entrypoints.first() {
            next.push(format!(
                "workspace related {} --by cochange",
                shell_hint(entrypoint)
            ));
        }
    }
    if let Some(command) = map.commands.get("test") {
        next.push(format!("workspace run {}", shell_hint(command)));
    }
    next
}

fn related_by_cochange(
    workspace: &Workspace,
    target: &str,
    max_commits: usize,
    max_files_per_commit: usize,
    max_results: usize,
    rank: RankingMethod,
    use_index: bool,
) -> Result<RelatedData> {
    let should_use_index = uses_cochange_index(use_index, rank);
    if !should_use_index && let Some(cli) = RelatedCli::detect() {
        let output = cli.query(
            &workspace.root,
            target,
            max_commits,
            max_files_per_commit,
            max_results,
            rank.as_str(),
        )?;
        return Ok(related_data_from_related_cli(
            target,
            output,
            max_commits,
            max_files_per_commit,
            max_results,
            rank,
        ));
    }

    if should_use_index {
        let index = read_cochange_index(workspace)?;
        let ranking = match rank {
            RankingMethod::Direct => rank_cochanges_from_index(&index, target, max_results),
            RankingMethod::Pagerank => {
                rank_cochanges_pagerank_from_index(&index, target, max_results)
            }
        };
        return Ok(cochange_related_data(
            RelatedDataMetadata::cochange(target, rank, RELATIONSHIP_SOURCE_COCHANGE_INDEX, true),
            RelationshipStats::from_cochange_index(&index, ranking.commits_matched),
            RelationshipLimits::from_cochange_index(&index),
            ranking.related,
        ));
    }

    let commits = git_recent_name_only_commits(workspace, max_commits)?;
    let ranking = rank_cochanges(&commits, target, max_files_per_commit, max_results);
    Ok(cochange_related_data(
        RelatedDataMetadata::cochange(target, rank, RELATIONSHIP_SOURCE_GIT_LOG, true),
        RelationshipStats::from_git_log(
            &commits,
            ranking.commits_matched,
            ranking.ignored_large_commits,
        ),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        ranking.related,
    ))
}

fn related_data_from_related_cli(
    target: &str,
    output: RelatedCliOutput,
    max_commits: usize,
    max_files_per_commit: usize,
    max_results: usize,
    rank: RankingMethod,
) -> RelatedData {
    let related = bounded_related_cli_files(output.related, max_results);
    let commits_matched = max_cochanged_commits(related.iter().map(|item| item.cochanged_commits));
    cochange_related_data(
        RelatedDataMetadata::cochange(
            target,
            rank,
            related_cli_relationship_source(&output.mode),
            true,
        ),
        RelationshipStats::from_related_cli(commits_matched),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        related,
    )
}

fn bounded_related_cli_files(items: Vec<RelatedCliItem>, max_results: usize) -> Vec<RelatedFile> {
    let mut related = Vec::new();
    for item in items {
        if related.len() >= max_results {
            break;
        }
        if let Some(file) = related_file_from_related_cli(item) {
            related.push(file);
        }
    }
    related
}

fn related_file_from_related_cli(item: RelatedCliItem) -> Option<RelatedFile> {
    let path = normalize_repo_path(&item.path);
    should_include_repo_file(&path).then(|| RelatedFile {
        path,
        score: round3(item.score),
        cochanged_commits: item.cochanges,
        weighted_cochanges: round3(item.weight),
        sample_commits: related_cli_sample_commits(&item.evidence),
    })
}

fn build_cochange_index(
    workspace: &Workspace,
    max_commits: usize,
    max_files_per_commit: usize,
) -> Result<CochangeIndex> {
    let commits = git_recent_name_only_commits(workspace, max_commits)?;
    let head = git_current_head(workspace)?;
    Ok(cochange_index_from_commits(
        &commits,
        max_commits,
        max_files_per_commit,
        head,
    ))
}

fn git_current_head(workspace: &Workspace) -> Result<Option<String>> {
    Ok(git_output(workspace, ["rev-parse", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn cochange_index_from_commits(
    commits: &[GitCommitFiles],
    max_commits: usize,
    max_files_per_commit: usize,
    head: Option<String>,
) -> CochangeIndex {
    let mut accumulators = BTreeMap::<(String, String), CochangeEdgeAccumulator>::new();
    let mut file_commit_counts = BTreeMap::<String, usize>::new();
    let mut commits_indexed = 0;
    let mut ignored_large_commits = 0;

    for (rank, commit) in commits.iter().enumerate() {
        let Some(files) = bounded_observable_commit_files(commit, max_files_per_commit.max(1))
        else {
            ignored_large_commits += 1;
            continue;
        };
        if files.len() < 2 {
            continue;
        }

        commits_indexed += 1;
        for file in &files {
            *file_commit_counts.entry(file.clone()).or_default() += 1;
        }

        let weight = cochange_commit_weight(rank, files.len());

        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let key = (files[i].clone(), files[j].clone());
                let accumulator = accumulators.entry(key).or_default();
                accumulator.cochanged_commits += 1;
                accumulator.weighted_cochanges += weight;
                push_sample_commit(&mut accumulator.sample_commits, short_commit(&commit.hash));
            }
        }
    }

    let edges = accumulators
        .into_iter()
        .map(|((a, b), item)| CochangeEdge {
            a,
            b,
            cochanged_commits: item.cochanged_commits,
            weighted_cochanges: round3(item.weighted_cochanges),
            sample_commits: item.sample_commits,
        })
        .collect();

    CochangeIndex {
        version: 1,
        generated_at_unix_ms: now_ms(),
        head,
        max_commits,
        max_files_per_commit,
        commits_scanned: commits.len(),
        commits_indexed,
        ignored_large_commits,
        file_commit_counts,
        edges,
    }
}

fn bounded_observable_commit_files(
    commit: &GitCommitFiles,
    max_files_per_commit: usize,
) -> Option<Vec<String>> {
    let mut files = BTreeSet::new();
    for file in &commit.files {
        let file = normalize_repo_path(file);
        if !should_include_repo_file(&file) {
            continue;
        }
        files.insert(file);
        if files.len() > max_files_per_commit {
            return None;
        }
    }
    Some(files.into_iter().collect())
}

fn cochange_index_status(workspace: &Workspace) -> IndexStatusData {
    let path = workspace.cochange_index_path();
    let path_label = workspace.relative(&path);
    if !workspace.is_git_repo {
        return empty_index_status(
            false,
            path_label,
            INDEX_STATUS_NOT_GIT_REPO,
            false,
            false,
            None,
            None,
        );
    }

    let current_head = git_current_head(workspace).ok().flatten();
    if !path.exists() {
        return empty_index_status(
            true,
            path_label,
            INDEX_STATUS_MISSING,
            false,
            false,
            current_head,
            None,
        );
    }

    match read_cochange_index(workspace) {
        Ok(index) => readable_index_status(path_label, current_head, index),
        Err(error) => empty_index_status(
            true,
            path_label,
            INDEX_STATUS_INVALID,
            true,
            false,
            current_head,
            Some(error.to_string()),
        ),
    }
}

fn readable_index_status(
    path: String,
    current_head: Option<String>,
    index: CochangeIndex,
) -> IndexStatusData {
    let fresh = current_head.is_some() && current_head == index.head;
    IndexStatusData {
        is_repo: true,
        path,
        exists: true,
        readable: true,
        status: index_freshness_status(fresh).to_string(),
        fresh,
        current_head,
        index_head: index.head,
        generated_at_unix_ms: Some(index.generated_at_unix_ms),
        max_commits: Some(index.max_commits),
        max_files_per_commit: Some(index.max_files_per_commit),
        commits_scanned: Some(index.commits_scanned),
        commits_indexed: Some(index.commits_indexed),
        ignored_large_commits: Some(index.ignored_large_commits),
        file_count: Some(index.file_commit_counts.len()),
        edge_count: Some(index.edges.len()),
        error: None,
    }
}

fn index_freshness_status(fresh: bool) -> &'static str {
    if fresh {
        INDEX_STATUS_FRESH
    } else {
        INDEX_STATUS_STALE
    }
}

fn empty_index_status(
    is_repo: bool,
    path: String,
    status: &str,
    exists: bool,
    readable: bool,
    current_head: Option<String>,
    error: Option<String>,
) -> IndexStatusData {
    IndexStatusData {
        is_repo,
        path,
        exists,
        readable,
        status: status.to_string(),
        fresh: false,
        current_head,
        index_head: None,
        generated_at_unix_ms: None,
        max_commits: None,
        max_files_per_commit: None,
        commits_scanned: None,
        commits_indexed: None,
        ignored_large_commits: None,
        file_count: None,
        edge_count: None,
        error,
    }
}

fn read_cochange_index(workspace: &Workspace) -> Result<CochangeIndex> {
    let path = workspace.cochange_index_path();
    read_cochange_index_from_path(&path)
}

fn read_cochange_index_from_path(path: &Path) -> Result<CochangeIndex> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to read co-change index {}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("failed to parse co-change index {}", path.display()))
}

fn write_cochange_index(path: &Path, index: &CochangeIndex) -> Result<()> {
    let temp_path = temp_sibling_path(path, "cochange-index")?;
    write_cochange_index_temp(&temp_path, index)?;
    if let Err(error) = fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace co-change index {}", path.display()))
    {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

fn temp_sibling_path(path: &Path, prefix: &str) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("co-change index path has no file name: {}", path.display()))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.tmp", new_id(prefix)));
    Ok(path.with_file_name(temp_name))
}

fn write_cochange_index_temp(path: &Path, index: &CochangeIndex) -> Result<()> {
    let file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to create temporary co-change index {}",
                    path.display()
                )
            });
        }
    };
    let mut writer = BufWriter::new(file);
    let result = (|| {
        serde_json::to_writer_pretty(&mut writer, index).with_context(|| {
            format!(
                "failed to serialize temporary co-change index {}",
                path.display()
            )
        })?;
        writer.flush().with_context(|| {
            format!(
                "failed to flush temporary co-change index {}",
                path.display()
            )
        })?;
        let file = writer.into_inner().with_context(|| {
            format!(
                "failed to finish temporary co-change index {}",
                path.display()
            )
        })?;
        file.sync_all().with_context(|| {
            format!(
                "failed to sync temporary co-change index {}",
                path.display()
            )
        })
    })();

    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn impact_by_cochange(
    workspace: &Workspace,
    max_commits: usize,
    max_files_per_commit: usize,
    max_results: usize,
    rank: RankingMethod,
    use_index: bool,
) -> Result<ImpactData> {
    let seed_files = git_changed_files(workspace)?;
    let seed_summary = SeedFileSummary::from_seed_files(&seed_files, MAX_CHANGED_FILES);
    let should_use_index = uses_cochange_index(use_index, rank);
    if !should_use_index
        && let Some(cli) = RelatedCli::detect()
        && let Some(data) = impact_by_related_cli(
            workspace,
            &cli,
            &seed_files,
            max_commits,
            max_files_per_commit,
            max_results,
            rank,
        )?
    {
        return Ok(data);
    }

    if should_use_index {
        let index = read_cochange_index(workspace)?;
        let ranking = match rank {
            RankingMethod::Direct => {
                rank_cochange_impact_from_index(&index, &seed_files, max_results)
            }
            RankingMethod::Pagerank => {
                rank_cochange_impact_pagerank_from_index(&index, &seed_files, max_results)
            }
        };
        return Ok(cochange_impact_data(
            ImpactDataMetadata::cochange(rank, RELATIONSHIP_SOURCE_COCHANGE_INDEX, true),
            seed_summary,
            RelationshipStats::from_cochange_index(&index, ranking.commits_matched),
            RelationshipLimits::from_cochange_index(&index),
            ranking.impacted,
        ));
    }

    let commits = git_recent_name_only_commits(workspace, max_commits)?;
    let ranking = rank_cochange_impact(&commits, &seed_files, max_files_per_commit, max_results);

    Ok(cochange_impact_data(
        ImpactDataMetadata::cochange(rank, RELATIONSHIP_SOURCE_GIT_LOG, true),
        seed_summary,
        RelationshipStats::from_git_log(
            &commits,
            ranking.commits_matched,
            ranking.ignored_large_commits,
        ),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        ranking.impacted,
    ))
}

fn impact_by_related_cli(
    workspace: &Workspace,
    cli: &RelatedCli,
    seed_files: &[String],
    max_commits: usize,
    max_files_per_commit: usize,
    max_results: usize,
    rank: RankingMethod,
) -> Result<Option<ImpactData>> {
    if seed_files.is_empty() {
        return Ok(None);
    }

    let seed_summary = SeedFileSummary::from_seed_files(seed_files, MAX_CHANGED_FILES);
    let seed_set = normalized_seed_file_set(seed_files);
    let mut accumulators = BTreeMap::<String, RelatedCliImpactAccumulator>::new();
    for seed in seed_files {
        let output = cli.query(
            &workspace.root,
            seed,
            max_commits,
            max_files_per_commit,
            max_results
                .saturating_add(seed_files.len())
                .max(max_results),
            rank.as_str(),
        )?;
        for item in output.related {
            let path = normalize_repo_path(&item.path);
            if !should_include_repo_file(&path) || seed_set.contains(&path) {
                continue;
            }
            let accumulator = accumulators.entry(path).or_default();
            accumulator.score += item.score;
            accumulator.cochanged_commits += item.cochanges;
            accumulator.weighted_cochanges += item.weight;
            accumulator.seed_files.insert(seed.clone());
            push_related_cli_sample_commits(&mut accumulator.sample_commits, &item.evidence);
        }
    }

    let max_score = max_rank_weight(accumulators.values().map(|item| item.score));
    let mut impacted = Vec::new();
    for (path, item) in accumulators {
        push_bounded_sorted(
            &mut impacted,
            impact_file_from_related_cli_accumulator(path, item, max_score),
            max_results,
            compare_impact_by_score,
        );
    }

    let commits_matched = max_cochanged_commits(impacted.iter().map(|item| item.cochanged_commits));

    Ok(Some(cochange_impact_data(
        ImpactDataMetadata::cochange(rank, related_cli_aggregate_relationship_source(rank), true),
        seed_summary,
        RelationshipStats::from_related_cli(commits_matched),
        RelationshipLimits::from_options(max_commits, max_files_per_commit),
        impacted,
    )))
}

fn git_recent_name_only_commits(
    workspace: &Workspace,
    max_commits: usize,
) -> Result<Vec<GitCommitFiles>> {
    let mut child = Command::new("git")
        .current_dir(&workspace.root)
        .args([
            "log".to_string(),
            "--format=commit:%H".to_string(),
            "--name-only".to_string(),
            format!("--max-count={}", max_commits.max(1)),
            "--".to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git log")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture git log stdout"))?;
    let stderr_reader = capture_child_stderr(&mut child, "git log stderr", MAX_CAPTURED_OUTPUT)?;

    let commits_result = read_git_log_name_only(stdout);
    let status = child.wait().context("failed to wait for git log")?;
    let stderr = join_captured_output_reader(stderr_reader, "git log stderr")?;
    let commits = commits_result?;
    if !status.success() {
        bail!("git log failed: {}", stderr.text.trim());
    }
    Ok(commits)
}

fn git_changed_files(workspace: &Workspace) -> Result<Vec<String>> {
    let mut files = BTreeSet::new();
    collect_git_name_only(workspace, ["diff", "--name-only"], &mut files)?;
    collect_git_name_only(workspace, ["diff", "--cached", "--name-only"], &mut files)?;

    stream_git_status_entries(workspace, |code, path| {
        if code == "??" && should_include_repo_file(&path) {
            files.insert(path);
        }
    })?;

    Ok(files.into_iter().collect())
}

fn collect_git_name_only<const N: usize>(
    workspace: &Workspace,
    args: [&str; N],
    files: &mut BTreeSet<String>,
) -> Result<()> {
    stream_git_name_only_paths(workspace, args, |path| {
        if should_include_repo_file(&path) {
            files.insert(path);
        }
    })?;
    Ok(())
}

fn workspace_arg_path(workspace: &Workspace, path: &Path) -> Result<String> {
    let resolved = workspace.resolve_path(path);
    let normalized_root = normalize_lexical_path(&workspace.root);
    let normalized_path = normalize_lexical_path(&resolved);
    let relative = normalized_path
        .strip_prefix(&normalized_root)
        .map_err(|_| {
            anyhow!(
                "path {} is outside workspace root {}",
                normalized_path.display(),
                normalized_root.display()
            )
        })?;
    if relative.as_os_str().is_empty() {
        bail!(
            "path {} resolves to workspace root {}",
            normalized_path.display(),
            normalized_root.display()
        );
    }
    Ok(normalize_repo_path(&relative.to_string_lossy()))
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
fn parse_git_log_name_only(output: &str) -> Vec<GitCommitFiles> {
    let mut state = GitLogNameOnlyState::default();
    for line in output.lines() {
        state.push_line(line);
    }
    state.finish()
}

fn read_git_log_name_only<R: Read>(reader: R) -> Result<Vec<GitCommitFiles>> {
    let mut reader = BufReader::new(reader);
    let mut state = GitLogNameOnlyState::default();
    let mut line_number = 1usize;

    while let Some(line) = read_bounded_output_line(
        &mut reader,
        line_number,
        MAX_GIT_OUTPUT_LINE_BYTES,
        "git log output",
    )? {
        line_number += 1;
        if line.exceeded {
            bail!(
                "git log output line {} exceeded {} bytes",
                line.line_number,
                MAX_GIT_OUTPUT_LINE_BYTES
            );
        }
        let line = String::from_utf8_lossy(&line.bytes);
        state.push_line(&line);
    }

    Ok(state.finish())
}

fn stream_git_name_only_paths<I, S, F>(workspace: &Workspace, args: I, mut on_path: F) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    F: FnMut(String),
{
    let mut child = Command::new("git")
        .current_dir(&workspace.root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture git stdout"))?;
    let stderr_reader = capture_child_stderr(&mut child, "git stderr", MAX_CAPTURED_OUTPUT)?;

    let paths_result = stream_git_name_only_paths_from_reader(stdout, &mut on_path);
    let status = child.wait().context("failed to wait for git")?;
    let stderr = join_captured_output_reader(stderr_reader, "git stderr")?;
    paths_result?;
    if !status.success() {
        bail!("git failed: {}", stderr.text.trim());
    }
    Ok(())
}

fn git_output_name_only_limited<I, S>(
    workspace: &Workspace,
    args: I,
    max_files: usize,
) -> Result<BoundedFileList>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new("git")
        .current_dir(&workspace.root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture git stdout"))?;
    let stderr_reader = capture_child_stderr(&mut child, "git stderr", MAX_CAPTURED_OUTPUT)?;

    let paths_result = read_git_name_only_paths_limited(stdout, max_files);
    let status = child.wait().context("failed to wait for git")?;
    let stderr = join_captured_output_reader(stderr_reader, "git stderr")?;
    let paths = paths_result?;
    if !status.success() {
        bail!("git failed: {}", stderr.text.trim());
    }
    Ok(paths)
}

#[cfg(test)]
fn read_git_name_only_paths<R: Read>(reader: R) -> Result<Vec<String>> {
    read_git_name_only_paths_limited(reader, usize::MAX).map(|paths| paths.files)
}

fn read_git_name_only_paths_limited<R: Read>(
    reader: R,
    max_files: usize,
) -> Result<BoundedFileList> {
    let mut reader = BufReader::new(reader);
    let mut line_number = 1usize;
    let mut paths = BoundedPathAccumulator::new(max_files);

    while let Some(line) = read_bounded_output_line(
        &mut reader,
        line_number,
        MAX_GIT_OUTPUT_LINE_BYTES,
        "git name-only output",
    )? {
        line_number += 1;
        if line.exceeded {
            bail!(
                "git name-only output line {} exceeded {} bytes",
                line.line_number,
                MAX_GIT_OUTPUT_LINE_BYTES
            );
        }
        let line = String::from_utf8_lossy(&line.bytes);
        if let Some(path) = git_name_only_path(&line) {
            paths.push(path);
        }
    }

    Ok(paths.into_file_list())
}

fn stream_git_name_only_paths_from_reader<R: Read, F>(reader: R, on_path: &mut F) -> Result<()>
where
    F: FnMut(String),
{
    let mut reader = BufReader::new(reader);
    let mut line_number = 1usize;

    while let Some(line) = read_bounded_output_line(
        &mut reader,
        line_number,
        MAX_GIT_OUTPUT_LINE_BYTES,
        "git name-only output",
    )? {
        line_number += 1;
        if line.exceeded {
            bail!(
                "git name-only output line {} exceeded {} bytes",
                line.line_number,
                MAX_GIT_OUTPUT_LINE_BYTES
            );
        }
        let line = String::from_utf8_lossy(&line.bytes);
        if let Some(path) = git_name_only_path(&line) {
            on_path(path);
        }
    }

    Ok(())
}

#[cfg(test)]
fn git_name_only_paths(output: &str) -> Vec<String> {
    output.lines().filter_map(git_name_only_path).collect()
}

fn git_name_only_path(line: &str) -> Option<String> {
    let raw = line.trim();
    if raw.is_empty() {
        return None;
    }

    let decoded = if raw.starts_with('"') {
        let (path, rest) = unquote_git_path(raw)?;
        if !rest.trim().is_empty() {
            return None;
        }
        path
    } else {
        raw.to_string()
    };
    let normalized = normalize_repo_path(&decoded);
    (!normalized.is_empty()).then_some(normalized)
}

fn git_status_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let path = raw.rsplit_once(" -> ").map_or(raw, |(_, path)| path);
    git_name_only_path(path)
}

fn rank_cochanges(
    commits: &[GitCommitFiles],
    target: &str,
    max_files_per_commit: usize,
    max_results: usize,
) -> CochangeRanking {
    let target = normalize_repo_path(target);
    let mut accumulators = BTreeMap::<String, CochangeAccumulator>::new();
    let mut commits_matched = 0;
    let mut ignored_large_commits = 0;

    for (rank, commit) in commits.iter().enumerate() {
        if !commit_mentions_normalized_path(commit, &target) {
            continue;
        }

        commits_matched += 1;

        let Some(files) = bounded_normalized_commit_files(commit, max_files_per_commit.max(1))
        else {
            ignored_large_commits += 1;
            continue;
        };

        let weight = cochange_commit_weight(rank, files.len());

        for file in files {
            if file == target {
                continue;
            }
            let accumulator = accumulators.entry(file).or_default();
            accumulator.cochanged_commits += 1;
            accumulator.weighted_cochanges += weight;
            push_sample_commit(&mut accumulator.sample_commits, short_commit(&commit.hash));
        }
    }

    let max_weight = max_rank_weight(accumulators.values().map(|item| item.weighted_cochanges));
    let mut related = Vec::new();
    for (path, item) in accumulators {
        push_bounded_sorted(
            &mut related,
            related_file_from_accumulator(path, item, max_weight),
            max_results,
            compare_related_by_weight,
        );
    }

    CochangeRanking {
        related,
        commits_matched,
        ignored_large_commits,
    }
}

fn bounded_normalized_commit_files(
    commit: &GitCommitFiles,
    max_files_per_commit: usize,
) -> Option<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for file in &commit.files {
        let file = normalize_repo_path(file);
        if file.is_empty() {
            continue;
        }
        files.insert(file);
        if files.len() > max_files_per_commit {
            return None;
        }
    }
    Some(files)
}

fn commit_mentions_normalized_path(commit: &GitCommitFiles, target: &str) -> bool {
    !target.is_empty()
        && commit
            .files
            .iter()
            .any(|file| normalize_repo_path(file) == target)
}

fn rank_cochanges_from_index(
    index: &CochangeIndex,
    target: &str,
    max_results: usize,
) -> CochangeRanking {
    let target = normalize_repo_path(target);
    let max_weight = max_rank_weight(
        index
            .edges
            .iter()
            .filter(|edge| edge.a == target || edge.b == target)
            .map(|edge| edge.weighted_cochanges),
    );
    let mut related = Vec::new();

    for edge in &index.edges {
        let path = if edge.a == target {
            edge.b.clone()
        } else if edge.b == target {
            edge.a.clone()
        } else {
            continue;
        };

        push_bounded_sorted(
            &mut related,
            related_file_from_edge(path, edge, max_weight),
            max_results,
            compare_related_by_weight,
        );
    }

    CochangeRanking {
        related,
        commits_matched: indexed_file_commit_count(index, &target),
        ignored_large_commits: 0,
    }
}

fn rank_cochanges_pagerank_from_index(
    index: &CochangeIndex,
    target: &str,
    max_results: usize,
) -> CochangeRanking {
    let target = normalize_repo_path(target);
    let seeds = BTreeSet::from([target.clone()]);
    let hits = personalized_pagerank(index, &seeds, 40, 0.85);
    let edge_lookup = cochange_edge_lookup(index);
    let mut related = hits
        .into_iter()
        .map(|hit| {
            let direct_edge = find_cochange_edge(&edge_lookup, &target, &hit.path);
            related_file_from_pagerank_hit(hit, direct_edge)
        })
        .collect::<Vec<_>>();

    sort_and_truncate(&mut related, max_results, compare_related_by_score);

    CochangeRanking {
        related,
        commits_matched: indexed_file_commit_count(index, &target),
        ignored_large_commits: 0,
    }
}

fn rank_cochange_impact(
    commits: &[GitCommitFiles],
    seed_files: &[String],
    max_files_per_commit: usize,
    max_results: usize,
) -> ImpactRanking {
    let seed_files = normalized_seed_file_set(seed_files);
    let mut accumulators = BTreeMap::<String, ImpactAccumulator>::new();
    let mut commits_matched = 0;
    let mut ignored_large_commits = 0;

    if seed_files.is_empty() {
        return ImpactRanking {
            impacted: vec![],
            commits_matched,
            ignored_large_commits,
        };
    }

    for (rank, commit) in commits.iter().enumerate() {
        if !commit_mentions_observable_seed(commit, &seed_files) {
            continue;
        }

        commits_matched += 1;

        let Some(files) = bounded_observable_commit_files(commit, max_files_per_commit.max(1))
        else {
            ignored_large_commits += 1;
            continue;
        };

        let matched_seed_count = files
            .iter()
            .filter(|file| seed_files.contains(*file))
            .count();
        let weight =
            cochange_commit_weight(rank, files.len()) * impact_seed_weight(matched_seed_count);
        let matched_seeds = files
            .iter()
            .filter(|file| seed_files.contains(*file))
            .cloned()
            .collect::<Vec<_>>();

        for file in files {
            if seed_files.contains(&file) {
                continue;
            }
            let accumulator = accumulators.entry(file).or_default();
            accumulator.cochanged_commits += 1;
            accumulator.weighted_cochanges += weight;
            accumulator.seed_files.extend(matched_seeds.iter().cloned());
            push_sample_commit(&mut accumulator.sample_commits, short_commit(&commit.hash));
        }
    }

    let max_weight = max_rank_weight(accumulators.values().map(|item| item.weighted_cochanges));
    let mut impacted = Vec::new();
    for (path, item) in accumulators {
        push_bounded_sorted(
            &mut impacted,
            impact_file_from_accumulator(path, item, max_weight),
            max_results,
            compare_impact_by_weight,
        );
    }

    ImpactRanking {
        impacted,
        commits_matched,
        ignored_large_commits,
    }
}

fn commit_mentions_observable_seed(commit: &GitCommitFiles, seed_files: &BTreeSet<String>) -> bool {
    !seed_files.is_empty()
        && commit.files.iter().any(|file| {
            let path = normalize_repo_path(file);
            should_include_repo_file(&path) && seed_files.contains(&path)
        })
}

fn rank_cochange_impact_from_index(
    index: &CochangeIndex,
    seed_files: &[String],
    max_results: usize,
) -> ImpactRanking {
    let seed_files = normalized_seed_file_set(seed_files);
    let mut accumulators = BTreeMap::<String, ImpactAccumulator>::new();

    for edge in &index.edges {
        let relation = match (seed_files.contains(&edge.a), seed_files.contains(&edge.b)) {
            (true, false) => Some((edge.b.clone(), edge.a.clone())),
            (false, true) => Some((edge.a.clone(), edge.b.clone())),
            _ => None,
        };
        let Some((candidate, seed)) = relation else {
            continue;
        };

        let accumulator = accumulators.entry(candidate).or_default();
        accumulator.cochanged_commits += edge.cochanged_commits;
        accumulator.weighted_cochanges += edge.weighted_cochanges;
        accumulator.seed_files.insert(seed);
        for commit in &edge.sample_commits {
            if !push_unique_sample_commit(&mut accumulator.sample_commits, commit) {
                break;
            }
        }
    }

    let max_weight = max_rank_weight(accumulators.values().map(|item| item.weighted_cochanges));
    let mut impacted = Vec::new();
    for (path, item) in accumulators {
        push_bounded_sorted(
            &mut impacted,
            impact_file_from_accumulator(path, item, max_weight),
            max_results,
            compare_impact_by_weight,
        );
    }

    let commits_matched = indexed_seed_commit_count(index, &seed_files);

    ImpactRanking {
        impacted,
        commits_matched,
        ignored_large_commits: 0,
    }
}

fn rank_cochange_impact_pagerank_from_index(
    index: &CochangeIndex,
    seed_files: &[String],
    max_results: usize,
) -> ImpactRanking {
    let seed_files = normalized_seed_file_set(seed_files);
    let hits = personalized_pagerank(index, &seed_files, 40, 0.85);
    let edge_lookup = cochange_edge_lookup(index);
    let mut impacted = hits
        .into_iter()
        .map(|hit| impact_file_from_pagerank_hit(hit, &seed_files, &edge_lookup))
        .collect::<Vec<_>>();

    sort_and_truncate(&mut impacted, max_results, compare_impact_by_score);
    let commits_matched = indexed_seed_commit_count(index, &seed_files);

    ImpactRanking {
        impacted,
        commits_matched,
        ignored_large_commits: 0,
    }
}

fn personalized_pagerank(
    index: &CochangeIndex,
    seed_files: &BTreeSet<String>,
    iterations: usize,
    damping: f64,
) -> Vec<PageRankHit> {
    if seed_files.is_empty() || index.edges.is_empty() {
        return vec![];
    }

    let mut graph = BTreeMap::<String, Vec<(String, f64)>>::new();
    for edge in &index.edges {
        let weight = edge.weighted_cochanges.max(0.0);
        if weight == 0.0 {
            continue;
        }
        graph
            .entry(edge.a.clone())
            .or_default()
            .push((edge.b.clone(), weight));
        graph
            .entry(edge.b.clone())
            .or_default()
            .push((edge.a.clone(), weight));
    }
    for seed in seed_files {
        graph.entry(seed.clone()).or_default();
    }

    let outbound_weights = graph
        .iter()
        .map(|(node, neighbors)| {
            (
                node.clone(),
                neighbors.iter().map(|(_, weight)| *weight).sum::<f64>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let seed_probability = 1.0 / seed_files.len() as f64;
    let mut personalization = BTreeMap::<String, f64>::new();
    for seed in seed_files {
        personalization.insert(seed.clone(), seed_probability);
    }
    let mut rank = personalization.clone();

    for _ in 0..iterations {
        let mut next = BTreeMap::<String, f64>::new();
        for (node, seed_rank) in &personalization {
            next.insert(node.clone(), (1.0 - damping) * seed_rank);
        }

        let mut dangling_rank = 0.0;
        for (node, neighbors) in &graph {
            let node_rank = rank.get(node).copied().unwrap_or_default();
            if neighbors.is_empty() {
                dangling_rank += node_rank;
                continue;
            }

            let total_weight = outbound_weights.get(node).copied().unwrap_or_default();
            if total_weight == 0.0 {
                dangling_rank += node_rank;
                continue;
            }

            for (neighbor, weight) in neighbors {
                *next.entry(neighbor.clone()).or_default() +=
                    damping * node_rank * (*weight / total_weight);
            }
        }

        if dangling_rank > 0.0 {
            for (node, seed_rank) in &personalization {
                *next.entry(node.clone()).or_default() += damping * dangling_rank * seed_rank;
            }
        }
        rank = next;
    }

    let max_score = max_rank_weight(
        rank.iter()
            .filter(|(path, _)| !seed_files.contains(*path))
            .map(|(_, score)| *score),
    );
    if max_score == 0.0 {
        return vec![];
    }

    let mut hits = rank
        .into_iter()
        .filter(|(path, score)| !seed_files.contains(path) && *score > 0.0)
        .map(|(path, score)| PageRankHit {
            path,
            score: score / max_score,
        })
        .collect::<Vec<_>>();
    hits.sort_by(compare_pagerank_hit_by_score);
    hits
}

fn cochange_edge_lookup(index: &CochangeIndex) -> BTreeMap<(String, String), &CochangeEdge> {
    let mut lookup = BTreeMap::new();
    for edge in &index.edges {
        lookup
            .entry(ordered_edge_key(&edge.a, &edge.b))
            .or_insert(edge);
    }
    lookup
}

fn find_cochange_edge<'a>(
    lookup: &BTreeMap<(String, String), &'a CochangeEdge>,
    a: &str,
    b: &str,
) -> Option<&'a CochangeEdge> {
    let a = normalize_repo_path(a);
    let b = normalize_repo_path(b);
    lookup.get(&ordered_edge_key(&a, &b)).copied()
}

fn ordered_edge_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn compare_related_by_weight(a: &RelatedFile, b: &RelatedFile) -> std::cmp::Ordering {
    b.weighted_cochanges
        .total_cmp(&a.weighted_cochanges)
        .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
        .then_with(|| a.path.cmp(&b.path))
}

fn compare_related_by_score(a: &RelatedFile, b: &RelatedFile) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
        .then_with(|| a.path.cmp(&b.path))
}

fn compare_impact_by_weight(a: &ImpactFile, b: &ImpactFile) -> std::cmp::Ordering {
    b.weighted_cochanges
        .total_cmp(&a.weighted_cochanges)
        .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
        .then_with(|| a.path.cmp(&b.path))
}

fn compare_impact_by_score(a: &ImpactFile, b: &ImpactFile) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
        .then_with(|| a.path.cmp(&b.path))
}

fn compare_pagerank_hit_by_score(a: &PageRankHit, b: &PageRankHit) -> std::cmp::Ordering {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.path.cmp(&b.path))
}

fn related_evidence(data: &RelatedData) -> Vec<Evidence> {
    data.related
        .iter()
        .take(MAX_EVIDENCE_ITEMS)
        .map(|file| Evidence {
            path: file.path.clone(),
            lines: None,
            reason: related_evidence_reason(data, file),
        })
        .collect()
}

fn related_evidence_reason(data: &RelatedData, file: &RelatedFile) -> String {
    if is_pagerank_only_hit(&data.ranking, file.cochanged_commits) {
        pagerank_related_evidence_reason(&data.target, file.score)
    } else {
        direct_related_evidence_reason(&data.target, file.cochanged_commits, &file.sample_commits)
    }
}

fn pagerank_related_evidence_reason(target: &str, score: f64) -> String {
    pagerank_evidence_reason(target, score)
}

fn direct_related_evidence_reason(
    target: &str,
    cochanged_commits: usize,
    sample_commits: &[String],
) -> String {
    direct_evidence_reason(target, cochanged_commits, sample_commits)
}

fn impact_evidence(data: &ImpactData) -> Vec<Evidence> {
    data.impacted
        .iter()
        .take(MAX_EVIDENCE_ITEMS)
        .map(|file| Evidence {
            path: file.path.clone(),
            lines: None,
            reason: impact_evidence_reason(data, file),
        })
        .collect()
}

fn impact_evidence_reason(data: &ImpactData, file: &ImpactFile) -> String {
    if is_pagerank_only_hit(&data.ranking, file.cochanged_commits) {
        pagerank_impact_evidence_reason(&file.seed_files, file.score)
    } else {
        direct_impact_evidence_reason(
            &file.seed_files,
            file.cochanged_commits,
            &file.sample_commits,
        )
    }
}

fn is_pagerank_only_hit(ranking: &str, cochanged_commits: usize) -> bool {
    ranking == RANK_PAGERANK && cochanged_commits == 0
}

fn pagerank_impact_evidence_reason(seed_files: &[String], score: f64) -> String {
    pagerank_evidence_reason(&seed_files_evidence_subject(seed_files), score)
}

fn pagerank_evidence_reason(subject: &str, score: f64) -> String {
    format!("reached from {subject} through the co-change graph; pagerank score {score:.3}")
}

fn direct_impact_evidence_reason(
    seed_files: &[String],
    cochanged_commits: usize,
    sample_commits: &[String],
) -> String {
    direct_evidence_reason(
        &seed_files_evidence_subject(seed_files),
        cochanged_commits,
        sample_commits,
    )
}

fn seed_files_evidence_subject(seed_files: &[String]) -> String {
    format!("seed file(s) {}", join_or_none(seed_files))
}

fn direct_evidence_reason(
    subject: &str,
    cochanged_commits: usize,
    sample_commits: &[String],
) -> String {
    format!(
        "changed with {subject} in {cochanged_commits} commit(s); samples: {}",
        join_or_none(sample_commits)
    )
}

fn relationship_source(use_index: bool) -> &'static str {
    if use_index {
        RELATIONSHIP_SOURCE_COCHANGE_INDEX
    } else {
        RELATIONSHIP_SOURCE_GIT_LOG
    }
}

fn relationship_source_for_options(use_index: bool, rank: RankingMethod) -> &'static str {
    relationship_source(uses_cochange_index(use_index, rank))
}

fn related_cli_relationship_source(mode: &str) -> String {
    format!("{RELATIONSHIP_SOURCE_RELATED_CLI}:{mode}")
}

fn related_cli_aggregate_relationship_source(rank: RankingMethod) -> String {
    format!(
        "{}:{}:aggregate",
        RELATIONSHIP_SOURCE_RELATED_CLI,
        rank.as_str()
    )
}

fn max_cochanged_commits(commits: impl IntoIterator<Item = usize>) -> usize {
    commits.into_iter().max().unwrap_or(0)
}

fn uses_cochange_index(use_index: bool, rank: RankingMethod) -> bool {
    use_index || rank == RankingMethod::Pagerank
}

fn normalize_repo_path(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn should_include_repo_file(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !has_windows_drive_prefix(path)
        && path != LOG_DIR
        && !path.starts_with(&format!("{LOG_DIR}/"))
        && !path.starts_with(".git/")
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn short_commit(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn push_sample_commit(sample_commits: &mut Vec<String>, commit: String) -> bool {
    if sample_commits.len() >= MAX_SAMPLE_COMMITS {
        return false;
    }
    sample_commits.push(commit);
    true
}

fn push_unique_sample_commit(sample_commits: &mut Vec<String>, commit: &str) -> bool {
    if sample_commits.len() >= MAX_SAMPLE_COMMITS {
        return false;
    }
    if !sample_commits.iter().any(|existing| existing == commit) {
        sample_commits.push(commit.to_string());
    }
    true
}

fn related_cli_sample_commits(evidence: &[RelatedCliEvidence]) -> Vec<String> {
    let mut sample_commits = Vec::new();
    push_related_cli_sample_commits(&mut sample_commits, evidence);
    sample_commits
}

fn push_related_cli_sample_commits(
    sample_commits: &mut Vec<String>,
    evidence: &[RelatedCliEvidence],
) {
    for item in evidence {
        if !push_sample_commit(sample_commits, short_commit(&item.hash)) {
            break;
        }
    }
}

fn cochange_commit_weight(rank: usize, file_count: usize) -> f64 {
    let recency_weight = 1.0 / (1.0 + rank as f64 / 50.0);
    let size_weight = 1.0 / (file_count.max(2) as f64 + 1.0).ln();
    recency_weight * size_weight
}

fn impact_seed_weight(matched_seed_count: usize) -> f64 {
    1.0 + (matched_seed_count.saturating_sub(1) as f64 * 0.25)
}

fn normalized_seed_file_set(seed_files: &[String]) -> BTreeSet<String> {
    seed_files
        .iter()
        .map(|file| normalize_repo_path(file))
        .filter(|file| !file.is_empty())
        .collect()
}

fn indexed_seed_commit_count(index: &CochangeIndex, seed_files: &BTreeSet<String>) -> usize {
    seed_files
        .iter()
        .map(|file| indexed_file_commit_count(index, file))
        .sum()
}

fn indexed_file_commit_count(index: &CochangeIndex, file: &str) -> usize {
    index.file_commit_counts.get(file).copied().unwrap_or(0)
}

fn max_rank_weight(weights: impl IntoIterator<Item = f64>) -> f64 {
    weights.into_iter().fold(0.0, f64::max)
}

fn related_file_from_accumulator(
    path: String,
    item: CochangeAccumulator,
    max_weight: f64,
) -> RelatedFile {
    RelatedFile {
        path,
        score: normalized_rank_score(item.weighted_cochanges, max_weight),
        cochanged_commits: item.cochanged_commits,
        weighted_cochanges: round3(item.weighted_cochanges),
        sample_commits: item.sample_commits,
    }
}

fn related_file_from_edge(path: String, edge: &CochangeEdge, max_weight: f64) -> RelatedFile {
    RelatedFile {
        path,
        score: normalized_rank_score(edge.weighted_cochanges, max_weight),
        cochanged_commits: edge.cochanged_commits,
        weighted_cochanges: edge.weighted_cochanges,
        sample_commits: edge.sample_commits.clone(),
    }
}

fn related_file_from_pagerank_hit(
    hit: PageRankHit,
    direct_edge: Option<&CochangeEdge>,
) -> RelatedFile {
    RelatedFile {
        path: hit.path,
        score: round3(hit.score),
        cochanged_commits: direct_edge
            .map(|edge| edge.cochanged_commits)
            .unwrap_or_default(),
        weighted_cochanges: direct_edge
            .map(|edge| edge.weighted_cochanges)
            .unwrap_or_default(),
        sample_commits: direct_edge
            .map(|edge| edge.sample_commits.clone())
            .unwrap_or_default(),
    }
}

fn impact_file_from_accumulator(
    path: String,
    item: ImpactAccumulator,
    max_weight: f64,
) -> ImpactFile {
    ImpactFile {
        path,
        score: normalized_rank_score(item.weighted_cochanges, max_weight),
        cochanged_commits: item.cochanged_commits,
        weighted_cochanges: round3(item.weighted_cochanges),
        seed_files: item.seed_files.into_iter().collect(),
        sample_commits: item.sample_commits,
    }
}

fn impact_file_from_related_cli_accumulator(
    path: String,
    item: RelatedCliImpactAccumulator,
    max_score: f64,
) -> ImpactFile {
    ImpactFile {
        path,
        score: normalized_rank_score(item.score, max_score),
        cochanged_commits: item.cochanged_commits,
        weighted_cochanges: round3(item.weighted_cochanges),
        seed_files: item.seed_files.into_iter().collect(),
        sample_commits: item.sample_commits,
    }
}

fn impact_file_from_pagerank_hit(
    hit: PageRankHit,
    seed_files: &BTreeSet<String>,
    edge_lookup: &BTreeMap<(String, String), &CochangeEdge>,
) -> ImpactFile {
    let mut direct_commits = 0usize;
    let mut direct_weight = 0.0f64;
    let mut direct_seeds = BTreeSet::new();
    let mut sample_commits = Vec::new();

    for seed in seed_files {
        if let Some(edge) = find_cochange_edge(edge_lookup, seed, &hit.path) {
            direct_commits += edge.cochanged_commits;
            direct_weight += edge.weighted_cochanges;
            direct_seeds.insert(seed.clone());
            for commit in &edge.sample_commits {
                if !push_unique_sample_commit(&mut sample_commits, commit) {
                    break;
                }
            }
        }
    }

    ImpactFile {
        path: hit.path,
        score: round3(hit.score),
        cochanged_commits: direct_commits,
        weighted_cochanges: round3(direct_weight),
        seed_files: if direct_seeds.is_empty() {
            seed_files.iter().cloned().collect()
        } else {
            direct_seeds.into_iter().collect()
        },
        sample_commits,
    }
}

fn normalized_rank_score(weight: f64, max_weight: f64) -> f64 {
    if max_weight > 0.0 {
        round3(weight / max_weight)
    } else {
        0.0
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn git_summary(workspace: &Workspace) -> Result<GitSummary> {
    if !workspace.is_git_repo {
        return Ok(GitSummary {
            is_repo: false,
            branch: None,
            dirty_file_count: 0,
            untracked_file_count: 0,
            dirty_files: vec![],
            untracked_files: vec![],
            omitted_dirty_files: 0,
            omitted_untracked_files: 0,
        });
    }

    let branch = git_output(workspace, ["branch", "--show-current"])
        .ok()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty());
    let mut dirty_files = BoundedPathAccumulator::new(MAX_GIT_STATUS_FILES);
    let mut untracked_files = BoundedPathAccumulator::new(MAX_GIT_STATUS_FILES);

    stream_git_status_entries(workspace, |code, path| {
        if path == LOG_DIR || path.starts_with(&format!("{LOG_DIR}/")) {
            return;
        }
        if code == "??" {
            untracked_files.push(path);
        } else {
            dirty_files.push(path);
        }
    })?;

    Ok(GitSummary {
        is_repo: true,
        branch,
        dirty_file_count: dirty_files.total_count,
        untracked_file_count: untracked_files.total_count,
        omitted_dirty_files: dirty_files.omitted_count(),
        omitted_untracked_files: untracked_files.omitted_count(),
        dirty_files: dirty_files.paths,
        untracked_files: untracked_files.paths,
    })
}

fn stream_git_status_entries<F>(workspace: &Workspace, mut handle: F) -> Result<()>
where
    F: FnMut(&str, String),
{
    let mut child = Command::new("git")
        .current_dir(&workspace.root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git status")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture git status stdout"))?;
    let stderr_reader = capture_child_stderr(&mut child, "git status stderr", MAX_CAPTURED_OUTPUT)?;

    let read_result = read_git_status_stdout(stdout, &mut handle);
    let status = child.wait().context("failed to wait for git status")?;
    let stderr = join_captured_output_reader(stderr_reader, "git status stderr")?;
    read_result?;
    if !status.success() {
        bail!("git status failed: {}", stderr.text.trim());
    }
    Ok(())
}

fn read_git_status_stdout<R, F>(reader: R, handle: &mut F) -> Result<()>
where
    R: Read,
    F: FnMut(&str, String),
{
    let mut reader = BufReader::new(reader);
    let mut line_number = 1usize;

    while let Some(line) = read_bounded_output_line(
        &mut reader,
        line_number,
        MAX_GIT_OUTPUT_LINE_BYTES,
        "git status output",
    )? {
        line_number += 1;
        if line.exceeded {
            bail!(
                "git status output line {} exceeded {} bytes",
                line.line_number,
                MAX_GIT_OUTPUT_LINE_BYTES
            );
        }
        let line = String::from_utf8_lossy(&line.bytes);
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let Some(path) = git_status_path(&line[3..]) else {
            continue;
        };
        handle(code, path);
    }

    Ok(())
}

fn rg_search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
) -> Result<(Vec<SearchMatch>, usize, usize)> {
    let mut child = match Command::new("rg")
        .current_dir(&workspace.root)
        .args([
            "--json",
            "--line-number",
            "--column",
            "--color",
            "never",
            "--glob",
            "!.git/**",
            "--glob",
            "!.workspace/**",
            "--glob",
            "!target/**",
            "--glob",
            "!node_modules/**",
            "--",
            query,
            ".",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return fallback_text_search(workspace, query, max_results);
        }
        Err(error) => return Err(error).context("failed to run ripgrep"),
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture ripgrep stdout"))?;
    let stderr_reader = capture_child_stderr(&mut child, "ripgrep stderr", MAX_CAPTURED_OUTPUT)?;
    let search_result = parse_rg_json_output(stdout, max_results);
    let status = child.wait().context("failed to wait for ripgrep")?;
    let stderr = join_captured_output_reader(stderr_reader, "ripgrep stderr")?;

    if !status.success() && status.code() != Some(1) {
        bail!("ripgrep failed: {}", stderr.text.trim());
    }

    match search_result {
        Ok(result) => Ok(result),
        Err(error) if error.downcast_ref::<RipgrepJsonLineTooLarge>().is_some() => {
            fallback_text_search(workspace, query, max_results)
        }
        Err(error) => Err(error),
    }
}

fn parse_rg_json_output<R: Read>(
    reader: R,
    max_results: usize,
) -> Result<(Vec<SearchMatch>, usize, usize)> {
    let mut matches = Vec::new();
    let mut total_matches = 0usize;
    let mut truncated_match_texts = 0usize;
    let mut reader = BufReader::new(reader);
    let mut line_number = 1usize;
    let mut first_error = None;

    while let Some(line) = read_bounded_output_line(
        &mut reader,
        line_number,
        MAX_RG_JSON_LINE_BYTES,
        "ripgrep JSON output",
    )? {
        line_number += 1;
        if first_error.is_some() {
            continue;
        }
        if line.exceeded {
            first_error = Some(anyhow!(RipgrepJsonLineTooLarge {
                line_number: line.line_number,
                max_bytes: MAX_RG_JSON_LINE_BYTES,
            }));
            continue;
        }
        let line = match String::from_utf8(line.bytes) {
            Ok(line) => line,
            Err(error) => {
                first_error = Some(anyhow!("ripgrep JSON output is not valid UTF-8: {error}"));
                continue;
            }
        };
        if let Err(error) = parse_rg_json_line(
            &line,
            max_results,
            &mut matches,
            &mut total_matches,
            &mut truncated_match_texts,
        ) {
            first_error = Some(error);
        }
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok((matches, total_matches, truncated_match_texts))
}

fn read_bounded_output_line<R: BufRead>(
    reader: &mut R,
    line_number: usize,
    max_bytes: usize,
    label: &str,
) -> Result<Option<BoundedOutputLine>> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut saw_bytes = false;

    loop {
        let (bytes_to_consume, reached_line_end) = {
            let buffer = reader
                .fill_buf()
                .with_context(|| format!("failed to read {label} line {line_number}"))?;
            if buffer.is_empty() {
                if !saw_bytes {
                    return Ok(None);
                }
                break;
            }

            saw_bytes = true;
            let (segment, consume_len, segment_reaches_line_end) =
                match buffer.iter().position(|byte| *byte == b'\n') {
                    Some(newline_index) => (&buffer[..newline_index], newline_index + 1, true),
                    None => (buffer, buffer.len(), false),
                };
            let remaining = max_bytes.saturating_sub(bytes.len());
            if remaining > 0 {
                let bytes_to_store = remaining.min(segment.len());
                bytes.extend_from_slice(&segment[..bytes_to_store]);
            }
            if segment.len() > remaining {
                exceeded = true;
            }

            (consume_len, segment_reaches_line_end)
        };

        reader.consume(bytes_to_consume);
        if reached_line_end {
            break;
        }
    }

    if !exceeded && bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(Some(BoundedOutputLine {
        line_number,
        bytes,
        exceeded,
    }))
}

fn parse_rg_json_line(
    line: &str,
    max_results: usize,
    matches: &mut Vec<SearchMatch>,
    total_matches: &mut usize,
    truncated_match_texts: &mut usize,
) -> Result<bool> {
    let value: Value = serde_json::from_str(line).context("failed to parse ripgrep JSON")?;
    if value.get("type").and_then(Value::as_str) != Some("match") {
        return Ok(false);
    }
    *total_matches += 1;
    if matches.len() >= max_results {
        return Ok(true);
    }
    let data = value.get("data").unwrap_or(&Value::Null);
    let path = data
        .get("path")
        .and_then(|path| path.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_start_matches("./")
        .to_string();
    let raw_text = data
        .get("lines")
        .and_then(|lines| lines.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_end_matches('\n')
        .to_string();
    let (text, text_truncated) = truncate_search_match_text(&raw_text);
    if text_truncated {
        *truncated_match_texts += 1;
    }
    let line_number = data
        .get("line_number")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let column = data
        .get("submatches")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("start"))
        .and_then(Value::as_u64)
        .map(|start| start + 1)
        .unwrap_or_default();

    matches.push(SearchMatch {
        path,
        line: line_number,
        column,
        text,
    });
    Ok(true)
}

fn fallback_text_search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
) -> Result<(Vec<SearchMatch>, usize, usize)> {
    let mut matches = Vec::new();
    let mut total_matches = 0usize;
    let mut truncated_match_texts = 0usize;

    for entry in WalkDir::new(&workspace.root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.path() == workspace.root || should_descend(entry.path(), false))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel_path = workspace.relative(path);
        if !should_include_repo_file(&rel_path) {
            continue;
        }

        let remaining_results = max_results.saturating_sub(matches.len());
        let Ok(file_result) = fallback_text_search_file(path, &rel_path, query, remaining_results)
        else {
            continue;
        };
        total_matches += file_result.total_matches;
        truncated_match_texts += file_result.truncated_match_texts;
        matches.extend(file_result.matches);
    }

    Ok((matches, total_matches, truncated_match_texts))
}

fn fallback_text_search_file(
    path: &Path,
    rel_path: &str,
    query: &str,
    max_results: usize,
) -> Result<FallbackSearchResult> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let query_bytes = query.as_bytes();
    let mut result = FallbackSearchResult {
        matches: Vec::new(),
        total_matches: 0,
        truncated_match_texts: 0,
    };
    let mut line = FallbackLineSearch::with_display(1, max_results > 0);

    loop {
        let (bytes_to_consume, reached_line_end) = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                if line.pending_line_cr {
                    fallback_push_line_bytes(&mut line, b"\r", query_bytes)?;
                    line.pending_line_cr = false;
                }
                if line.saw_bytes {
                    fallback_finish_line(
                        &mut line,
                        rel_path,
                        query_bytes,
                        max_results,
                        &mut result,
                    )?;
                }
                break;
            }

            let (line_segment, consume_len, segment_reaches_line_end) =
                match buffer.iter().position(|byte| *byte == b'\n') {
                    Some(newline_index) => (&buffer[..newline_index], newline_index + 1, true),
                    None => (buffer, buffer.len(), false),
                };

            fallback_push_line_segment(
                &mut line,
                line_segment,
                segment_reaches_line_end,
                query_bytes,
            )?;

            if segment_reaches_line_end {
                fallback_finish_line(&mut line, rel_path, query_bytes, max_results, &mut result)?;
            }

            (consume_len, segment_reaches_line_end)
        };

        reader.consume(bytes_to_consume);
        if reached_line_end {
            line = FallbackLineSearch::with_display(
                line.line_number + 1,
                result.matches.len() < max_results,
            );
        }
    }

    Ok(result)
}

fn fallback_push_line_segment(
    line: &mut FallbackLineSearch,
    line_segment: &[u8],
    segment_reaches_line_end: bool,
    query: &[u8],
) -> Result<()> {
    if line.pending_line_cr {
        if line_segment.is_empty() && segment_reaches_line_end {
            line.pending_line_cr = false;
        } else {
            fallback_push_line_bytes(line, b"\r", query)?;
            line.pending_line_cr = false;
        }
    }

    let selected_segment = if line_segment.ends_with(b"\r") {
        if segment_reaches_line_end {
            &line_segment[..line_segment.len() - 1]
        } else {
            line.pending_line_cr = true;
            &line_segment[..line_segment.len() - 1]
        }
    } else {
        line_segment
    };

    fallback_push_line_bytes(line, selected_segment, query)
}

fn fallback_push_line_bytes(
    line: &mut FallbackLineSearch,
    bytes: &[u8],
    query: &[u8],
) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }

    line.saw_bytes = true;
    fallback_scan_query(line, bytes, query);
    fallback_append_display_bytes(line, bytes)?;
    line.byte_offset += bytes.len();
    Ok(())
}

fn fallback_scan_query(line: &mut FallbackLineSearch, bytes: &[u8], query: &[u8]) {
    if line.matched || query.is_empty() {
        return;
    }

    if let Some(index) = find_query_with_tail(&line.scan_tail, bytes, query) {
        line.matched = true;
        line.match_column =
            line.byte_offset.saturating_sub(line.scan_tail.len()) as u64 + index as u64 + 1;
    }

    replace_scan_tail(&mut line.scan_tail, bytes, query.len().saturating_sub(1));
}

fn find_query_with_tail(tail: &[u8], bytes: &[u8], query: &[u8]) -> Option<usize> {
    if query.is_empty() || query.len() > tail.len() + bytes.len() {
        return None;
    }

    let boundary_start = tail.len().saturating_sub(query.len().saturating_sub(1));
    for start in boundary_start..tail.len() {
        if start + query.len() <= tail.len() || start + query.len() > tail.len() + bytes.len() {
            continue;
        }
        if query_matches_virtual_window(tail, bytes, start, query) {
            return Some(start);
        }
    }

    bytes
        .windows(query.len())
        .position(|window| window == query)
        .map(|index| tail.len() + index)
}

fn query_matches_virtual_window(tail: &[u8], bytes: &[u8], start: usize, query: &[u8]) -> bool {
    query.iter().enumerate().all(|(offset, expected)| {
        let index = start + offset;
        let actual = if index < tail.len() {
            tail[index]
        } else {
            bytes[index - tail.len()]
        };
        actual == *expected
    })
}

fn replace_scan_tail(tail: &mut Vec<u8>, bytes: &[u8], max_tail_len: usize) {
    if max_tail_len == 0 {
        tail.clear();
        return;
    }

    if bytes.len() >= max_tail_len {
        tail.clear();
        tail.extend_from_slice(&bytes[bytes.len() - max_tail_len..]);
        return;
    }

    let old_tail_len = tail.len();
    let old_bytes_to_keep = max_tail_len.saturating_sub(bytes.len());
    if old_tail_len > old_bytes_to_keep {
        tail.drain(..old_tail_len - old_bytes_to_keep);
    }
    tail.extend_from_slice(bytes);
}

fn fallback_append_display_bytes(line: &mut FallbackLineSearch, bytes: &[u8]) -> Result<()> {
    line.pending_utf8.extend_from_slice(bytes);
    let valid_len = match std::str::from_utf8(&line.pending_utf8) {
        Ok(_) => line.pending_utf8.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(error) => bail!("file is not valid UTF-8: {error}"),
    };

    if valid_len == 0 {
        return Ok(());
    }

    if line.capture_display && !line.display_truncated {
        let valid_text = std::str::from_utf8(&line.pending_utf8[..valid_len])?;
        line.display_truncated = append_limited_text(
            &mut line.display_text,
            &mut line.display_char_count,
            MAX_SEARCH_MATCH_TEXT,
            valid_text,
        );
    }
    line.pending_utf8.drain(..valid_len);
    Ok(())
}

fn fallback_finish_line(
    line: &mut FallbackLineSearch,
    rel_path: &str,
    query: &[u8],
    max_results: usize,
    result: &mut FallbackSearchResult,
) -> Result<()> {
    ensure_no_pending_utf8(&line.pending_utf8)?;
    if query.is_empty() {
        line.matched = true;
        line.match_column = 1;
    }
    if !line.matched {
        return Ok(());
    }

    result.total_matches += 1;
    if result.matches.len() >= max_results {
        return Ok(());
    }
    if line.display_truncated {
        result.truncated_match_texts += 1;
    }
    result.matches.push(SearchMatch {
        path: rel_path.to_string(),
        line: line.line_number,
        column: line.match_column,
        text: std::mem::take(&mut line.display_text),
    });
    Ok(())
}

fn truncate_search_match_text(text: &str) -> (String, bool) {
    let truncated = text.chars().count() > MAX_SEARCH_MATCH_TEXT;
    (truncate_string(text, MAX_SEARCH_MATCH_TEXT), truncated)
}

fn parse_line_range(value: &str) -> Result<(usize, usize)> {
    let (start, end) = value
        .split_once(':')
        .ok_or_else(|| anyhow!("expected START:END"))?;
    let start = start.parse::<usize>().context("invalid start line")?;
    let end = end.parse::<usize>().context("invalid end line")?;
    if start == 0 || end == 0 || start > end {
        bail!("line range must be positive and START <= END");
    }
    Ok((start, end))
}

fn read_text_prefix_bounded(path: &Path) -> Result<ReadContent> {
    let mut file = fs::File::open(path)?;
    let mut output = String::new();
    let mut char_count = 0usize;
    let mut pending_utf8 = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        if append_bounded_utf8_bytes(
            &mut output,
            &mut char_count,
            &mut pending_utf8,
            &buffer[..bytes_read],
        )? {
            truncated = true;
            break;
        }
    }

    if !truncated {
        ensure_no_pending_utf8(&pending_utf8)?;
    }

    Ok(ReadContent {
        content: output,
        truncated,
    })
}

fn read_line_range_bounded(path: &Path, start: usize, end: usize) -> Result<ReadContent> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut output = String::new();
    let mut char_count = 0usize;
    let mut line_number = 1usize;
    let mut wrote_selected_line = false;
    let mut started_selected_line = false;
    let mut pending_utf8 = Vec::new();
    let mut pending_line_cr = false;
    let mut truncated = false;

    while line_number <= end {
        let line_is_selected = line_number >= start;
        let (bytes_to_consume, reached_line_end) = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                if started_selected_line {
                    ensure_no_pending_utf8(&pending_utf8)?;
                }
                break;
            }

            let (line_segment, consume_len, segment_reaches_line_end) =
                match buffer.iter().position(|byte| *byte == b'\n') {
                    Some(newline_index) => (&buffer[..newline_index], newline_index + 1, true),
                    None => (buffer, buffer.len(), false),
                };

            if line_is_selected {
                if !started_selected_line {
                    if wrote_selected_line
                        && append_bounded_text(&mut output, &mut char_count, "\n")
                    {
                        truncated = true;
                    }
                    wrote_selected_line = true;
                    started_selected_line = true;
                }

                if !truncated && pending_line_cr {
                    if line_segment.is_empty() && segment_reaches_line_end {
                        pending_line_cr = false;
                    } else if append_bounded_utf8_bytes(
                        &mut output,
                        &mut char_count,
                        &mut pending_utf8,
                        b"\r",
                    )? {
                        truncated = true;
                    } else {
                        pending_line_cr = false;
                    }
                }

                let selected_segment = if !truncated && line_segment.ends_with(b"\r") {
                    pending_line_cr = true;
                    &line_segment[..line_segment.len() - 1]
                } else {
                    line_segment
                };

                if !truncated
                    && append_bounded_utf8_bytes(
                        &mut output,
                        &mut char_count,
                        &mut pending_utf8,
                        selected_segment,
                    )?
                {
                    truncated = true;
                }

                if !truncated && segment_reaches_line_end {
                    pending_line_cr = false;
                    ensure_no_pending_utf8(&pending_utf8)?;
                }
            }

            (consume_len, segment_reaches_line_end)
        };

        reader.consume(bytes_to_consume);
        if truncated {
            break;
        }
        if reached_line_end {
            line_number += 1;
            started_selected_line = false;
        }
    }

    Ok(ReadContent {
        content: output,
        truncated,
    })
}

fn append_bounded_text(output: &mut String, char_count: &mut usize, text: &str) -> bool {
    append_limited_text(output, char_count, MAX_READ_CONTENT, text)
}

fn append_limited_text(
    output: &mut String,
    char_count: &mut usize,
    max_chars: usize,
    text: &str,
) -> bool {
    for ch in text.chars() {
        if *char_count >= max_chars {
            output.push_str(OUTPUT_TRUNCATED_MARKER);
            return true;
        }
        output.push(ch);
        *char_count += 1;
    }
    false
}

fn append_bounded_utf8_bytes(
    output: &mut String,
    char_count: &mut usize,
    pending_utf8: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<bool> {
    pending_utf8.extend_from_slice(bytes);
    let valid_len = match std::str::from_utf8(pending_utf8) {
        Ok(_) => pending_utf8.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(error) => bail!("file is not valid UTF-8: {error}"),
    };

    if valid_len == 0 {
        return Ok(false);
    }

    let truncated = {
        let valid_text = std::str::from_utf8(&pending_utf8[..valid_len])?;
        append_bounded_text(output, char_count, valid_text)
    };
    pending_utf8.drain(..valid_len);
    Ok(truncated)
}

fn ensure_no_pending_utf8(pending_utf8: &[u8]) -> Result<()> {
    if pending_utf8.is_empty() {
        return Ok(());
    }

    match std::str::from_utf8(pending_utf8) {
        Ok(_) => Ok(()),
        Err(error) => bail!("file is not valid UTF-8: {error}"),
    }
}

#[cfg(test)]
fn extract_patch_files(patch_content: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in patch_content.lines() {
        collect_patch_file_line(line, &mut files);
    }
    files.into_iter().collect()
}

fn extract_patch_files_from_path(path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)?;
    extract_patch_files_from_reader(file)
}

fn extract_patch_files_from_reader<R: Read>(reader: R) -> Result<Vec<String>> {
    let mut reader = BufReader::new(reader);
    let mut files = BTreeSet::new();
    let mut line_number = 1usize;

    while let Some(line) =
        read_bounded_output_line(&mut reader, line_number, MAX_PATCH_LINE_BYTES, "patch")?
    {
        line_number += 1;
        if !patch_line_has_file_header_prefix(&line.bytes) {
            continue;
        }
        if line.exceeded {
            bail!(
                "patch file header line {} exceeded {} bytes",
                line.line_number,
                MAX_PATCH_LINE_BYTES
            );
        }
        let line = std::str::from_utf8(&line.bytes).with_context(|| {
            format!("patch header line {} is not valid UTF-8", line.line_number)
        })?;
        collect_patch_file_line(line, &mut files);
    }

    Ok(files.into_iter().collect())
}

fn patch_line_has_file_header_prefix(line: &[u8]) -> bool {
    line.starts_with(b"+++ ")
        || line.starts_with(b"--- ")
        || line.starts_with(b"rename from ")
        || line.starts_with(b"rename to ")
        || line.starts_with(b"diff --git ")
}

fn collect_patch_file_line(line: &str, files: &mut BTreeSet<String>) {
    if let Some(path) = line.strip_prefix("+++ ").and_then(clean_patch_path) {
        files.insert(path);
    } else if let Some(path) = line.strip_prefix("--- ").and_then(clean_patch_path) {
        files.insert(path);
    } else if let Some(path) = line.strip_prefix("rename from ").and_then(clean_patch_path) {
        files.insert(path);
    } else if let Some(path) = line.strip_prefix("rename to ").and_then(clean_patch_path) {
        files.insert(path);
    } else if let Some((old_path, new_path)) = diff_git_paths(line) {
        if let Some(path) = clean_diff_git_path(&old_path) {
            files.insert(path);
        }
        if let Some(path) = clean_diff_git_path(&new_path) {
            files.insert(path);
        }
    }
}

fn validate_patch_targets(files_changed: &[String]) -> Result<()> {
    if let Some(path) = files_changed
        .iter()
        .find(|path| !should_include_repo_file(path))
    {
        bail!("patch target {path:?} is outside observable workspace files");
    }
    Ok(())
}

fn diff_git_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    if rest.starts_with('"') {
        let (old_path, rest) = unquote_git_path(rest)?;
        let (new_path, rest) = unquote_git_path(rest.trim_start())?;
        if rest.trim().is_empty() {
            return Some((old_path, new_path));
        }
        return None;
    }

    let rest = rest.strip_prefix("a/")?;
    let (old_path, new_path) = rest.rsplit_once(" b/")?;
    Some((old_path.to_string(), new_path.to_string()))
}

fn clean_patch_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let path = if raw.starts_with('"') {
        unquote_git_path(raw)?.0
    } else {
        raw.split_once('\t')
            .map_or(raw, |(path, _)| path)
            .to_string()
    };
    clean_diff_git_path(&path)
}

fn clean_diff_git_path(raw: &str) -> Option<String> {
    let path = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    if path.is_empty() || path == "/dev/null" {
        None
    } else {
        Some(path.to_string())
    }
}

fn unquote_git_path(raw: &str) -> Option<(String, &str)> {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }

    let mut output = Vec::new();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let path = String::from_utf8(output).ok()?;
                return Some((path, &raw[index + 1..]));
            }
            b'\\' => {
                index += 1;
                if index >= bytes.len() {
                    return None;
                }
                match bytes[index] {
                    b'a' => output.push(0x07),
                    b'b' => output.push(0x08),
                    b'f' => output.push(0x0c),
                    b'n' => output.push(b'\n'),
                    b'r' => output.push(b'\r'),
                    b't' => output.push(b'\t'),
                    b'v' => output.push(0x0b),
                    b'\\' => output.push(b'\\'),
                    b'"' => output.push(b'"'),
                    b'0'..=b'7' => {
                        let mut value = bytes[index] - b'0';
                        for _ in 0..2 {
                            if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0'..=b'7')
                            {
                                break;
                            }
                            index += 1;
                            value = value * 8 + (bytes[index] - b'0');
                        }
                        output.push(value);
                    }
                    byte => output.push(byte),
                }
            }
            byte => output.push(byte),
        }
        index += 1;
    }
    None
}

fn run_git_apply<const N: usize>(
    workspace: &Workspace,
    patch_path: &Path,
    extra_args: [&str; N],
) -> Result<()> {
    let mut command = Command::new("git");
    command.current_dir(&workspace.root).arg("apply");
    for arg in extra_args {
        command.arg(arg);
    }
    command.arg(patch_path);
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git apply")?;
    let output =
        wait_for_captured_command(child, "git apply", MAX_CAPTURED_OUTPUT, MAX_CAPTURED_OUTPUT)?;
    if !output.status.success() {
        let message = if output.stderr.text.trim().is_empty() {
            output.stdout.text.trim()
        } else {
            output.stderr.text.trim()
        };
        bail!("git apply failed: {message}");
    }
    Ok(())
}

fn git_output<I, S>(workspace: &Workspace, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output_bounded(workspace, args, MAX_CAPTURED_OUTPUT)?;
    if output.truncated {
        bail!("git output exceeded {} bytes", MAX_CAPTURED_OUTPUT);
    }
    Ok(output.text)
}

fn git_observable_diff_name_only(
    workspace: &Workspace,
    max_files: usize,
) -> Result<BoundedFileList> {
    let mut args = vec!["diff"];
    if git_current_head(workspace)?.is_some() {
        args.push("HEAD");
    }
    args.push("--name-only");
    args.extend(["--", ".", ":(exclude).workspace/**"]);
    git_output_name_only_limited(workspace, args, max_files)
}

fn git_observable_diff_output_bounded<const N: usize>(
    workspace: &Workspace,
    extra_args: [&str; N],
    max_bytes: usize,
) -> Result<CapturedOutput> {
    let mut args = vec!["diff"];
    if git_current_head(workspace)?.is_some() {
        args.push("HEAD");
    }
    args.extend(extra_args);
    args.extend(["--", ".", ":(exclude).workspace/**"]);
    git_output_bounded(workspace, args, max_bytes)
}

fn git_output_bounded<I, S>(
    workspace: &Workspace,
    args: I,
    max_stdout_bytes: usize,
) -> Result<CapturedOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let child = Command::new("git")
        .current_dir(&workspace.root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    let output = wait_for_captured_command(child, "git", max_stdout_bytes, MAX_CAPTURED_OUTPUT)?;
    if !output.status.success() {
        bail!("git failed: {}", output.stderr.text.trim());
    }
    Ok(output.stdout)
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

fn wait_for_captured_command(
    mut child: Child,
    command_name: &str,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> Result<CapturedCommandOutput> {
    let stdout_name = format!("{command_name} stdout");
    let stderr_name = format!("{command_name} stderr");
    let stdout_reader = capture_child_stdout(&mut child, &stdout_name, max_stdout_bytes)?;
    let stderr_reader = capture_child_stderr(&mut child, &stderr_name, max_stderr_bytes)?;
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {command_name}"))?;
    let stdout = join_captured_output_reader(stdout_reader, &stdout_name)?;
    let stderr = join_captured_output_reader(stderr_reader, &stderr_name)?;

    Ok(CapturedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn capture_child_stdout(
    child: &mut Child,
    stream_name: &str,
    max_bytes: usize,
) -> Result<CapturedOutputReader> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture {stream_name}"))?;
    Ok(std::thread::spawn(move || {
        read_captured_output_with_limit(stdout, max_bytes)
    }))
}

fn capture_child_stderr(
    child: &mut Child,
    stream_name: &str,
    max_bytes: usize,
) -> Result<CapturedOutputReader> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture {stream_name}"))?;
    Ok(std::thread::spawn(move || {
        read_captured_output_with_limit(stderr, max_bytes)
    }))
}

fn join_captured_output_reader(
    reader: CapturedOutputReader,
    stream_name: &str,
) -> Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| anyhow!("{stream_name} reader thread panicked"))?
}

fn read_captured_output_with_limit<R: Read>(
    mut reader: R,
    max_bytes: usize,
) -> Result<CapturedOutput> {
    let mut stored = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .context("failed to read command output")?;
        if bytes_read == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(stored.len());
        if remaining > 0 {
            let bytes_to_store = remaining.min(bytes_read);
            stored.extend_from_slice(&buffer[..bytes_to_store]);
        }
        if bytes_read > remaining {
            truncated = true;
        }
    }

    let mut text = String::from_utf8_lossy(&stored).into_owned();
    if truncated {
        text.push_str(OUTPUT_TRUNCATED_MARKER);
    }
    Ok(CapturedOutput { text, truncated })
}

fn append_operation_log(workspace: &Workspace, record: OperationLogRecord<'_>) -> Result<()> {
    let entry = operation_log_entry(record);
    let line = serde_json::to_string(&entry)?;
    use std::io::Write;
    let mut file = open_log_for_append(workspace)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn operation_log_entry(record: OperationLogRecord<'_>) -> LogEntry {
    operation_log_entry_with_metadata(record, new_id("op"), now_ms())
}

fn operation_log_entry_with_metadata(
    record: OperationLogRecord<'_>,
    id: String,
    timestamp_unix_ms: u128,
) -> LogEntry {
    LogEntry {
        id,
        timestamp_unix_ms,
        kind: record.kind.to_string(),
        op: record.op.to_string(),
        scope: truncate_inline(record.scope, MAX_LOG_SCOPE),
        summary: truncate_inline(record.summary, MAX_LOG_SUMMARY),
        transaction_id: record.transaction_id.map(ToOwned::to_owned),
    }
}

fn output_best_effort_logged_observation<T, F>(
    workspace: &Workspace,
    json: bool,
    op: &str,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    let _ = append_operation_log(
        workspace,
        OperationLogRecord::observe_observation(op, observation),
    );
    output_observation(json, observation, print_human)
}

fn output_required_logged_observation<T, F>(
    workspace: &Workspace,
    json: bool,
    op: &str,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    output_recorded_observation(
        workspace,
        json,
        OperationLogRecord::observe_observation(op, observation),
        observation,
        print_human,
    )
}

fn output_changed_observation<T, F>(
    workspace: &Workspace,
    json: bool,
    op: &str,
    transaction_id: &str,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    output_recorded_observation(
        workspace,
        json,
        OperationLogRecord::change_observation(op, observation, transaction_id),
        observation,
        print_human,
    )
}

fn output_changed_observation_with_summary<T, F>(
    workspace: &Workspace,
    json: bool,
    op: &str,
    summary: &str,
    transaction_id: &str,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    output_recorded_observation(
        workspace,
        json,
        OperationLogRecord::change_observation_summary(op, observation, summary, transaction_id),
        observation,
        print_human,
    )
}

fn output_verified_observation<T, F>(
    workspace: &Workspace,
    json: bool,
    op: &str,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    output_recorded_observation(
        workspace,
        json,
        OperationLogRecord::verify_observation(op, observation),
        observation,
        print_human,
    )
}

struct OperationLogRecord<'a> {
    kind: &'a str,
    op: &'a str,
    scope: &'a str,
    summary: &'a str,
    transaction_id: Option<&'a str>,
}

impl<'a> OperationLogRecord<'a> {
    fn observe(op: &'a str, scope: &'a str, summary: &'a str) -> Self {
        Self {
            kind: LOG_KIND_OBSERVE,
            op,
            scope,
            summary,
            transaction_id: None,
        }
    }

    fn observe_observation<T: Serialize>(op: &'a str, observation: &'a Observation<T>) -> Self {
        Self::observe(op, &observation.scope, &observation.summary)
    }

    fn change(op: &'a str, scope: &'a str, summary: &'a str, transaction_id: &'a str) -> Self {
        Self {
            kind: LOG_KIND_CHANGE,
            op,
            scope,
            summary,
            transaction_id: Some(transaction_id),
        }
    }

    fn change_observation<T: Serialize>(
        op: &'a str,
        observation: &'a Observation<T>,
        transaction_id: &'a str,
    ) -> Self {
        Self::change(op, &observation.scope, &observation.summary, transaction_id)
    }

    fn change_observation_summary<T: Serialize>(
        op: &'a str,
        observation: &'a Observation<T>,
        summary: &'a str,
        transaction_id: &'a str,
    ) -> Self {
        Self::change(op, &observation.scope, summary, transaction_id)
    }

    fn verify(op: &'a str, scope: &'a str, summary: &'a str) -> Self {
        Self {
            kind: LOG_KIND_VERIFY,
            op,
            scope,
            summary,
            transaction_id: None,
        }
    }

    fn verify_observation<T: Serialize>(op: &'a str, observation: &'a Observation<T>) -> Self {
        Self::verify(op, &observation.scope, &observation.summary)
    }
}

fn output_recorded_observation<T, F>(
    workspace: &Workspace,
    json: bool,
    record: OperationLogRecord<'_>,
    observation: &Observation<T>,
    print_human: F,
) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    append_operation_log(workspace, record)?;
    output_observation(json, observation, print_human)
}

fn ensure_log_writable(workspace: &Workspace) -> Result<()> {
    open_log_for_append(workspace).map(|_| ())
}

fn open_log_for_append(workspace: &Workspace) -> Result<fs::File> {
    let log_dir = workspace.root.join(LOG_DIR);
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(workspace.log_path())
        .with_context(|| format!("failed to open {}", workspace.log_path().display()))
}

fn read_log(workspace: &Workspace, limit: usize) -> Result<LogWindow> {
    let path = workspace.log_path();
    if !path.exists() {
        return Ok(LogWindow::default());
    }
    if !path.is_file() {
        bail!("failed to read log {}: not a file", path.display());
    }
    let file =
        fs::File::open(&path).with_context(|| format!("failed to read log {}", path.display()))?;
    read_log_entries(BufReader::new(file), limit)
        .with_context(|| format!("failed to parse operation log {}", path.display()))
}

fn read_log_entries<R: BufRead>(mut reader: R, limit: usize) -> Result<LogWindow> {
    if limit == 0 {
        return Ok(LogWindow::default());
    }

    let mut non_empty_lines = 0usize;
    let mut window = VecDeque::new();
    let mut line_number = 1usize;
    let mut line = PendingLogLine::new(line_number);

    loop {
        let (bytes_to_consume, reached_line_end) = {
            let buffer = reader
                .fill_buf()
                .with_context(|| format!("failed to read operation log line {line_number}"))?;
            if buffer.is_empty() {
                if line.saw_non_whitespace {
                    push_log_window_line(&mut window, limit, line.into_stored());
                    non_empty_lines += 1;
                }
                break;
            }

            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(newline_index) => {
                    line.push_segment(&buffer[..newline_index]);
                    (newline_index + 1, true)
                }
                None => {
                    line.push_segment(buffer);
                    (buffer.len(), false)
                }
            }
        };

        reader.consume(bytes_to_consume);
        if reached_line_end {
            if line.saw_non_whitespace {
                push_log_window_line(&mut window, limit, line.into_stored());
                non_empty_lines += 1;
            }
            line_number += 1;
            line = PendingLogLine::new(line_number);
        }
    }

    let omitted_lines = non_empty_lines.saturating_sub(window.len());
    Ok(LogWindow {
        entries: parse_log_entries(window)?,
        omitted_lines,
    })
}

fn push_log_window_line(window: &mut VecDeque<StoredLogLine>, limit: usize, line: StoredLogLine) {
    if window.len() == limit {
        window.pop_front();
    }
    window.push_back(line);
}

fn parse_log_entries<I>(lines: I) -> Result<Vec<LogEntry>>
where
    I: IntoIterator<Item = StoredLogLine>,
{
    lines
        .into_iter()
        .map(|line| {
            let line_number = line.line_number;
            if line.oversized {
                bail!(
                    "operation log line {} exceeded {} bytes",
                    line_number,
                    MAX_LOG_LINE_BYTES
                );
            }
            let text = String::from_utf8(line.bytes)
                .with_context(|| format!("operation log line {line_number} is not valid UTF-8"))?;
            serde_json::from_str::<LogEntry>(&text)
                .with_context(|| format!("invalid operation log JSON at line {line_number}"))
        })
        .collect()
}

fn output_observation<T, F>(json: bool, observation: &Observation<T>, print_human: F) -> Result<()>
where
    T: Serialize,
    F: FnOnce(&Observation<T>) -> Result<()>,
{
    if json {
        println!("{}", serde_json::to_string_pretty(observation)?);
        Ok(())
    } else {
        print_human(observation)
    }
}

fn print_map(observation: &Observation<WorkspaceMap>) -> Result<()> {
    let map = &observation.data;
    println!("Workspace Map");
    println!("  root: {}", map.root);
    println!(
        "  git: {}",
        if map.git.is_repo {
            format!(
                "branch {}, {} dirty, {} untracked",
                map.git.branch.as_deref().unwrap_or("unknown"),
                map.git.dirty_file_count,
                map.git.untracked_file_count
            )
        } else {
            SUMMARY_NOT_GIT_REPOSITORY.to_string()
        }
    );
    println!("  languages: {}", join_or_none(&map.stack.languages));
    println!(
        "  package managers: {}",
        join_or_none(&map.stack.package_managers)
    );
    println!("  frameworks: {}", join_or_none(&map.stack.frameworks));
    println!("  files: {}", map.stats.file_count);
    print_list("entrypoints", &map.structure.entrypoints);
    print_list("tests", &map.structure.tests);
    print_list("configs", &map.structure.configs);
    print_list("docs", &map.structure.docs);
    if !map.commands.is_empty() {
        println!("  commands:");
        for (name, command) in &map.commands {
            println!("    {name}: {command}");
        }
    }
    if !observation.next_observations.is_empty() {
        print_list("next", &observation.next_observations);
    }
    Ok(())
}

fn print_status(observation: &Observation<StatusData>) -> Result<()> {
    let data = &observation.data;
    println!("Workspace Status");
    println!("  root: {}", data.root);
    if data.git.is_repo {
        println!(
            "  branch: {}",
            data.git.branch.as_deref().unwrap_or("unknown")
        );
        print_list("dirty", &data.git.dirty_files);
        print_omitted_items(data.git.omitted_dirty_files, "dirty file(s)");
        print_list("untracked", &data.git.untracked_files);
        print_omitted_items(data.git.omitted_untracked_files, "untracked file(s)");
        println!("  index: {}", data.index_status.status);
        println!("  index fresh: {}", data.index_status.fresh);
        if let Some(edge_count) = data.index_status.edge_count {
            println!("  index edges: {}", edge_count);
        }
    } else {
        println!("  git: not a repository");
    }
    if !data.recent_operations.is_empty() {
        println!("  recent operations:");
        for entry in &data.recent_operations {
            println!(
                "    {} {} {} - {}",
                entry.id, entry.kind, entry.op, entry.summary
            );
        }
    }
    if let Some(error) = &data.recent_operations_error {
        println!("  recent operations error: {error}");
    }
    Ok(())
}

fn print_search(observation: &Observation<SearchData>) -> Result<()> {
    println!("{}", observation.summary);
    for item in &observation.data.matches {
        println!("{}:{}:{}: {}", item.path, item.line, item.column, item.text);
    }
    if observation.truncated {
        println!("results truncated");
    }
    Ok(())
}

fn print_index_cochange(observation: &Observation<IndexCochangeData>) -> Result<()> {
    let data = &observation.data;
    println!("{}", observation.summary);
    println!("  path: {}", data.path);
    println!("  head: {}", data.head.as_deref().unwrap_or("unknown"));
    println!("  scanned: {} commit(s)", data.commits_scanned);
    println!("  indexed: {} commit(s)", data.commits_indexed);
    println!("  ignored broad commits: {}", data.ignored_large_commits);
    println!("  files: {}", data.file_count);
    println!("  edges: {}", data.edge_count);
    Ok(())
}

fn print_index_status(observation: &Observation<IndexStatusData>) -> Result<()> {
    let data = &observation.data;
    println!("{}", observation.summary);
    println!("  path: {}", data.path);
    println!("  exists: {}", data.exists);
    println!("  readable: {}", data.readable);
    println!("  fresh: {}", data.fresh);
    if let Some(current_head) = &data.current_head {
        println!("  current head: {}", short_commit(current_head));
    }
    if let Some(index_head) = &data.index_head {
        println!("  index head: {}", short_commit(index_head));
    }
    if let Some(file_count) = data.file_count {
        println!("  files: {}", file_count);
    }
    if let Some(edge_count) = data.edge_count {
        println!("  edges: {}", edge_count);
    }
    if let Some(error) = &data.error {
        println!("  error: {}", error);
    }
    Ok(())
}

fn print_related(observation: &Observation<RelatedData>) -> Result<()> {
    let data = &observation.data;
    println!("{}", observation.summary);
    if !data.is_repo {
        return Ok(());
    }
    println!("  source: {}", data.relationship_source);
    println!("  ranking: {}", data.ranking);
    println!(
        "{}",
        relationship_scan_summary(
            data.commits_scanned,
            data.commits_matched,
            data.ignored_large_commits
        )
    );
    for file in &data.related {
        println!(
            "  {:.3}  {}  ({} co-change commit(s), samples: {})",
            file.score,
            file.path,
            file.cochanged_commits,
            join_or_none(&file.sample_commits)
        );
    }
    Ok(())
}

fn print_impact(observation: &Observation<ImpactData>) -> Result<()> {
    let data = &observation.data;
    println!("{}", observation.summary);
    if !data.is_repo {
        return Ok(());
    }
    println!("  source: {}", data.relationship_source);
    println!("  ranking: {}", data.ranking);
    print_list("seeds", &data.seed_files);
    if impact_seed_files_omitted(data) {
        print_omitted_items(data.omitted_seed_files, "seed file(s)");
    }
    println!(
        "{}",
        relationship_scan_summary(
            data.commits_scanned,
            data.commits_matched,
            data.ignored_large_commits
        )
    );
    for file in &data.impacted {
        println!(
            "  {:.3}  {}  ({} co-change commit(s), seeds: {}, samples: {})",
            file.score,
            file.path,
            file.cochanged_commits,
            join_or_none(&file.seed_files),
            join_or_none(&file.sample_commits)
        );
    }
    Ok(())
}

fn relationship_scan_summary(
    commits_scanned: usize,
    commits_matched: usize,
    ignored_large_commits: usize,
) -> String {
    format!(
        "  scanned: {commits_scanned} commit(s), matched: {commits_matched}, ignored broad commits: {ignored_large_commits}"
    )
}

fn print_read(observation: &Observation<ReadData>) -> Result<()> {
    print!("{}", observation.data.content);
    if needs_trailing_newline(&observation.data.content) {
        println!();
    }
    Ok(())
}

fn print_diff(observation: &Observation<DiffData>) -> Result<()> {
    let data = &observation.data;
    if !data.is_repo {
        println!("{}", data.summary);
        return Ok(());
    }
    if let Some(summary) = nonblank_trimmed_end(&data.summary) {
        println!("{summary}");
    }
    if let Some(patch) = data.patch.as_deref().and_then(nonblank_trimmed_end) {
        println!("{patch}");
    }
    Ok(())
}

fn print_patch(observation: &Observation<PatchData>) -> Result<()> {
    println!("{}", observation.summary);
    println!("  transaction: {}", observation.data.transaction_id);
    print_list("files", &observation.data.files_changed);
    print_omitted_items(observation.data.omitted_files, "file(s)");
    Ok(())
}

fn print_run(observation: &Observation<RunData>) -> Result<()> {
    let data = &observation.data;
    if !data.stdout.is_empty() {
        print!("{}", data.stdout);
        if needs_trailing_newline(&data.stdout) {
            println!();
        }
    }
    if !data.stderr.is_empty() {
        eprint!("{}", data.stderr);
        if needs_trailing_newline(&data.stderr) {
            eprintln!();
        }
    }
    println!("{}", observation.summary);
    Ok(())
}

fn print_log(observation: &Observation<LogData>) -> Result<()> {
    let data = &observation.data;
    if data.entries.is_empty() {
        println!("no operations recorded");
        return Ok(());
    }
    for entry in &data.entries {
        println!(
            "{} {} {} {} - {}",
            entry.timestamp_unix_ms, entry.kind, entry.op, entry.scope, entry.summary
        );
    }
    if log_lines_omitted(data) {
        println!("... {} older log line(s) omitted", data.omitted_lines);
    }
    Ok(())
}

fn print_rollback(observation: &Observation<RollbackData>) -> Result<()> {
    println!("{}", observation.summary);
    println!(
        "  rollback transaction: {}",
        observation.data.rollback_transaction_id
    );
    print_list("files", &observation.data.files_changed);
    print_omitted_items(observation.data.omitted_files, "file(s)");
    Ok(())
}

fn is_test_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.starts_with("tests/")
        || lower.contains("/tests/")
        || lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
}

fn print_list(label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    println!("  {label}:");
    for value in values.iter().take(20) {
        println!("    {value}");
    }
    if values.len() > 20 {
        println!("    ... {} more", values.len() - 20);
    }
}

fn print_omitted_items(count: usize, item_label: &str) {
    if let Some(message) = omitted_items_message(count, item_label) {
        println!("{message}");
    }
}

fn omitted_items_message(count: usize, item_label: &str) -> Option<String> {
    if count > 0 {
        Some(format!("    ... {count} more {item_label}"))
    } else {
        None
    }
}

fn needs_trailing_newline(text: &str) -> bool {
    !text.ends_with('\n')
}

fn nonblank_trimmed_end(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text.trim_end())
    }
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn workspace_read_command(path: &str) -> String {
    format!("workspace read {}", shell_hint(path))
}

fn workspace_read_lines_command(path: &str, start: u64, end: u64) -> String {
    format!("{} --lines {start}:{end}", workspace_read_command(path))
}

fn shell_hint(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '-' | '_'))
    {
        value.to_string()
    } else {
        let mut quoted = String::from("'");
        for ch in value.chars() {
            if ch == '\'' {
                quoted.push_str("'\\''");
            } else {
                quoted.push(ch);
            }
        }
        quoted.push('\'');
        quoted
    }
}

fn truncate_string(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(OUTPUT_TRUNCATED_MARKER);
    truncated
}

fn truncate_inline(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(INLINE_TRUNCATED_MARKER);
    truncated
}

fn new_id(prefix: &str) -> String {
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{}{:05}{:06}",
        now_ms(),
        std::process::id(),
        sequence
    )
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_ranges() {
        assert_eq!(parse_line_range("1:3").unwrap(), (1, 3));
        assert!(parse_line_range("3:1").is_err());
        assert!(parse_line_range("0:1").is_err());
        assert!(parse_line_range("abc").is_err());
    }

    #[test]
    fn shell_hints_quote_values_for_shell_reuse() {
        assert_eq!(shell_hint("src/main.rs"), "src/main.rs");
        assert_eq!(shell_hint("space name.txt"), "'space name.txt'");
        assert_eq!(shell_hint("weird$path.txt"), "'weird$path.txt'");
        assert_eq!(shell_hint("quote'name.txt"), "'quote'\\''name.txt'");
    }

    #[test]
    fn captured_command_helper_collects_and_bounds_output() {
        let mut command = Command::new(std::env::current_exe().expect("test exe should exist"));
        let child = command
            .arg("--help")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("test harness should run");

        let output = wait_for_captured_command(child, "test harness", 8, MAX_CAPTURED_OUTPUT)
            .expect("test harness output should be captured");

        assert!(output.status.success());
        assert!(output.stdout.truncated);
        assert!(output.stdout.text.contains("[output truncated]"));
        assert!(!output.stderr.truncated);
    }

    #[test]
    fn decodes_git_quoted_path_lines() {
        assert_eq!(git_name_only_path("src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(
            git_name_only_path("\"src/tab\\tname.txt\"").unwrap(),
            "src/tab\tname.txt"
        );
        assert_eq!(
            git_name_only_paths("src/a.rs\n\"src/tab\\tname.txt\"\n"),
            vec!["src/a.rs", "src/tab\tname.txt"]
        );
    }

    #[test]
    fn reads_git_name_only_paths_incrementally() {
        let paths =
            read_git_name_only_paths(std::io::Cursor::new("src/a.rs\n\"src/tab\\tname.txt\"\n"))
                .expect("name-only output should parse");

        assert_eq!(paths, vec!["src/a.rs", "src/tab\tname.txt"]);
    }

    #[test]
    fn streams_git_name_only_paths_incrementally() {
        let mut paths = Vec::new();
        stream_git_name_only_paths_from_reader(
            std::io::Cursor::new("\nsrc/a.rs\n\"src/tab\\tname.txt\"\n"),
            &mut |path| paths.push(path),
        )
        .expect("name-only output should stream");

        assert_eq!(paths, vec!["src/a.rs", "src/tab\tname.txt"]);
    }

    #[test]
    fn reads_limited_git_name_only_paths_and_counts_omitted() {
        let paths = read_git_name_only_paths_limited(
            std::io::Cursor::new("src/a.rs\nsrc/b.rs\nsrc/c.rs\n"),
            2,
        )
        .expect("name-only output should parse");

        assert_eq!(paths.files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(paths.total_files, 3);
        assert_eq!(paths.omitted_files, 1);
    }

    #[test]
    fn bounded_path_accumulator_counts_omitted_paths() {
        let mut paths = BoundedPathAccumulator::new(2);
        paths.push("src/a.rs".to_string());
        paths.push("src/b.rs".to_string());
        paths.push("src/c.rs".to_string());

        assert_eq!(paths.total_count, 3);
        assert_eq!(paths.paths, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(paths.omitted_count(), 1);

        let file_list = paths.into_file_list();
        assert_eq!(file_list.files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(file_list.total_files, 3);
        assert_eq!(file_list.omitted_files, 1);
    }

    #[test]
    fn rejects_oversized_git_name_only_lines() {
        let output = format!("{}\n", "x".repeat(MAX_GIT_OUTPUT_LINE_BYTES + 1));

        let Err(error) = read_git_name_only_paths(std::io::Cursor::new(output)) else {
            panic!("oversized git name-only line should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("git name-only output line 1"),
            "unexpected error: {error}"
        );
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn decodes_git_status_paths() {
        assert_eq!(
            git_status_path("\"src/tab\\tname.txt\"").unwrap(),
            "src/tab\tname.txt"
        );
        assert_eq!(
            git_status_path("\"old\\tname.txt\" -> \"new\\tname.txt\"").unwrap(),
            "new\tname.txt"
        );
    }

    #[test]
    fn reads_git_status_stdout_entries_incrementally() {
        let mut entries = Vec::new();
        read_git_status_stdout(
            std::io::Cursor::new(" M \"src/tab\\tname.txt\"\n?? new/file.rs\n"),
            &mut |code, path| entries.push((code.to_string(), path)),
        )
        .expect("status stdout should parse");

        assert_eq!(
            entries,
            vec![
                (" M".to_string(), "src/tab\tname.txt".to_string()),
                ("??".to_string(), "new/file.rs".to_string()),
            ]
        );
    }

    #[test]
    fn rejects_oversized_git_status_lines() {
        let output = format!(" M {}\n", "x".repeat(MAX_GIT_OUTPUT_LINE_BYTES + 1));
        let mut entries = Vec::new();

        let Err(error) = read_git_status_stdout(std::io::Cursor::new(output), &mut |code, path| {
            entries.push((code.to_string(), path));
        }) else {
            panic!("oversized git status line should fail");
        };
        let error = format!("{error:#}");

        assert!(entries.is_empty());
        assert!(
            error.contains("git status output line 1"),
            "unexpected error: {error}"
        );
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn extracts_patch_files() {
        let patch = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-old
+new
";
        assert_eq!(extract_patch_files(patch), vec!["src/main.rs"]);
    }

    #[test]
    fn extracts_patch_files_without_header_metadata() {
        let patch = "\
diff --git a/space name.txt b/space name.txt
--- a/space name.txt\t2026-05-24
+++ b/space name.txt\t2026-05-24
@@ -1 +1 @@
-old
+new
";
        assert_eq!(extract_patch_files(patch), vec!["space name.txt"]);
    }

    #[test]
    fn extracts_patch_files_from_rename_headers() {
        let patch = "\
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
";
        assert_eq!(
            extract_patch_files(patch),
            vec!["new name.txt", "old name.txt"]
        );
    }

    #[test]
    fn extracts_patch_files_from_diff_git_only_sections() {
        let patch = "\
diff --git a/assets/logo.png b/assets/logo.png
new file mode 100644
index 0000000..1234567
GIT binary patch
literal 0

diff --git a/old data.bin b/new data.bin
similarity index 100%
";
        assert_eq!(
            extract_patch_files(patch),
            vec!["assets/logo.png", "new data.bin", "old data.bin"]
        );
    }

    #[test]
    fn extracts_patch_files_from_quoted_git_paths() {
        let patch = "\
diff --git \"a/src/tab\\tname.txt\" \"b/src/tab\\tname.txt\"
new file mode 100644
index 0000000..587be6b
--- /dev/null
+++ \"b/src/tab\\tname.txt\"
@@ -0,0 +1 @@
+x
";
        assert_eq!(extract_patch_files(patch), vec!["src/tab\tname.txt"]);
    }

    #[test]
    fn extracts_patch_files_from_reader() {
        let patch = "\
diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
";
        let files = extract_patch_files_from_reader(std::io::Cursor::new(patch))
            .expect("patch reader should parse");

        assert_eq!(files, vec!["new name.txt", "old name.txt", "src/a.rs"]);
    }

    #[test]
    fn extract_patch_files_skips_oversized_body_lines() {
        let patch = format!(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-{}\n+new\n",
            "x".repeat(MAX_PATCH_LINE_BYTES + 1)
        );

        let files = extract_patch_files_from_reader(std::io::Cursor::new(patch))
            .expect("patch reader should parse");

        assert_eq!(files, vec!["a.txt"]);
    }

    #[test]
    fn extract_patch_files_rejects_oversized_header_lines() {
        let patch = format!("+++ b/{}\n", "x".repeat(MAX_PATCH_LINE_BYTES + 1));

        let Err(error) = extract_patch_files_from_reader(std::io::Cursor::new(patch)) else {
            panic!("oversized patch header should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("patch file header line 1"),
            "unexpected error: {error}"
        );
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn observed_changed_files_and_evidence_are_bounded() {
        let files = (0..(MAX_CHANGED_FILES + 2))
            .map(|index| format!("file_{index:03}.txt"))
            .collect::<Vec<_>>();

        let observed = observed_changed_files(&files);
        let evidence = changed_file_evidence(&files, EVIDENCE_REASON_PATCH_FILE_TARGET);

        assert_eq!(observed.files.len(), MAX_CHANGED_FILES);
        assert_eq!(observed.files[0], "file_000.txt");
        assert_eq!(
            observed.files[MAX_CHANGED_FILES - 1],
            format!("file_{:03}.txt", MAX_CHANGED_FILES - 1)
        );
        assert_eq!(observed.file_count, MAX_CHANGED_FILES + 2);
        assert_eq!(observed.omitted_files, 2);
        assert_eq!(evidence.len(), MAX_CHANGED_FILES);
        assert_eq!(evidence[0].path, "file_000.txt");
        assert_eq!(evidence[0].reason, EVIDENCE_REASON_PATCH_FILE_TARGET);
        assert!(evidence.iter().all(|item| item.lines.is_none()));
    }

    #[test]
    fn transaction_data_helpers_bound_files_and_paths() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let patch_path = temp.path().join("change.patch");
        let stored_patch = temp.path().join(TRANSACTION_DIR).join("tx-1.patch");
        let files = (0..(MAX_CHANGED_FILES + 1))
            .map(|index| format!("file_{index:03}.txt"))
            .collect::<Vec<_>>();

        let data = patch_data(&workspace, "tx-1", &patch_path, &stored_patch, &files);
        assert_eq!(data.transaction_id, "tx-1");
        assert_eq!(data.patch_file, "change.patch");
        assert_eq!(data.stored_patch, ".workspace/transactions/tx-1.patch");
        assert_eq!(data.file_count, MAX_CHANGED_FILES + 1);
        assert_eq!(data.files_changed.len(), MAX_CHANGED_FILES);
        assert_eq!(data.omitted_files, 1);
        assert!(transaction_files_truncated(data.omitted_files));

        let data = rollback_data(&workspace, "tx-1", "rb-1", &stored_patch, &files);
        assert_eq!(data.transaction_id, "tx-1");
        assert_eq!(data.rollback_transaction_id, "rb-1");
        assert_eq!(data.stored_patch, ".workspace/transactions/tx-1.patch");
        assert_eq!(data.file_count, MAX_CHANGED_FILES + 1);
        assert_eq!(data.files_changed.len(), MAX_CHANGED_FILES);
        assert_eq!(data.omitted_files, 1);
    }

    #[test]
    fn transaction_observation_helpers_preserve_contract() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let patch_path = temp.path().join("change.patch");
        let stored_patch = temp.path().join(TRANSACTION_DIR).join("tx-1.patch");
        let files = (0..(MAX_CHANGED_FILES + 1))
            .map(|index| format!("file_{index:03}.txt"))
            .collect::<Vec<_>>();

        let data = patch_data(&workspace, "tx-1", &patch_path, &stored_patch, &files);
        let observation = patch_observation(data, &files);
        assert_eq!(observation.kind, WORKSPACE_PATCH_KIND);
        assert_eq!(observation.scope, "change.patch");
        assert_eq!(
            observation.summary,
            format!(
                "applied patch transaction tx-1 touching {} file(s) (files truncated)",
                MAX_CHANGED_FILES + 1
            )
        );
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), MAX_CHANGED_FILES);
        assert_eq!(
            observation.evidence[0].reason,
            EVIDENCE_REASON_PATCH_FILE_TARGET
        );
        assert_eq!(
            observation.next_observations,
            patch_followup_observations("tx-1")
        );

        let data = rollback_data(&workspace, "tx-1", "rb-1", &stored_patch, &files);
        let observation = rollback_observation(data, &files);
        assert_eq!(observation.kind, WORKSPACE_ROLLBACK_KIND);
        assert_eq!(observation.scope, "tx-1");
        assert_eq!(
            observation.summary,
            format!(
                "rolled back transaction tx-1 touching {} file(s) (files truncated)",
                MAX_CHANGED_FILES + 1
            )
        );
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), MAX_CHANGED_FILES);
        assert_eq!(
            observation.evidence[0].reason,
            EVIDENCE_REASON_ROLLBACK_TARGET
        );
        assert_eq!(
            observation.next_observations,
            rollback_followup_observations()
        );
    }

    #[test]
    fn transaction_applied_observation_helpers_preserve_contract() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let patch_path = temp.path().join("change.patch");
        let stored_patch = temp.path().join(TRANSACTION_DIR).join("tx-1.patch");
        let files = vec!["src/main.rs".to_string(), "README.md".to_string()];
        let patch = AppliedPatchTransaction {
            transaction_id: "tx-1".to_string(),
            patch_path,
            stored_patch: stored_patch.clone(),
            files_changed: files.clone(),
        };

        let observation = patch_transaction_observation(&workspace, &patch);

        assert_eq!(observation.kind, WORKSPACE_PATCH_KIND);
        assert_eq!(observation.scope, "change.patch");
        assert_eq!(observation.data.transaction_id, "tx-1");
        assert_eq!(
            observation.data.stored_patch,
            ".workspace/transactions/tx-1.patch"
        );
        assert_eq!(observation.data.file_count, 2);
        assert_eq!(
            patch_log_summary(Some("custom summary".to_string()), &observation),
            "custom summary"
        );
        assert_eq!(patch_log_summary(None, &observation), observation.summary);

        let rollback = AppliedRollbackTransaction {
            rollback_transaction_id: "rb-1".to_string(),
            stored_patch,
            files_changed: files,
        };
        let rollback_observation = rollback_transaction_observation(&workspace, "tx-1", &rollback);

        assert_eq!(rollback_observation.kind, WORKSPACE_ROLLBACK_KIND);
        assert_eq!(rollback_observation.scope, "tx-1");
        assert_eq!(rollback_observation.data.transaction_id, "tx-1");
        assert_eq!(rollback_observation.data.rollback_transaction_id, "rb-1");
        assert_eq!(rollback_observation.data.file_count, 2);
    }

    #[test]
    fn read_next_observations_include_existing_files_only() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::write(temp.path().join("a.txt"), "a").expect("file should be written");
        fs::create_dir(temp.path().join("dir")).expect("directory should be created");
        for index in 0..6 {
            fs::write(temp.path().join(format!("file_{index}.txt")), "x")
                .expect("file should be written");
        }
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let paths = [
            "missing.txt",
            "a.txt",
            "dir",
            "file_0.txt",
            "file_1.txt",
            "file_2.txt",
            "file_3.txt",
            "file_4.txt",
            "file_5.txt",
        ];

        let next = read_next_observations(&workspace, paths);

        assert_eq!(
            next,
            vec![
                "workspace read a.txt",
                "workspace read file_0.txt",
                "workspace read file_1.txt",
                "workspace read file_2.txt",
                "workspace read file_3.txt",
            ]
        );
    }

    #[test]
    fn read_observation_helpers_report_requested_content() {
        let read = ObservedRead {
            data: ReadData {
                path: "src/main.rs".to_string(),
                lines: Some("3:5".to_string()),
                content: "content".to_string(),
            },
            content_truncated: true,
        };

        let evidence = read_evidence(&read.data);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].path, "src/main.rs");
        assert_eq!(evidence[0].lines.as_deref(), Some("3:5"));
        assert_eq!(evidence[0].reason, EVIDENCE_REASON_REQUESTED_FILE_CONTENT);

        let observation = read_observation(read);
        assert_eq!(observation.kind, WORKSPACE_READ_KIND);
        assert_eq!(observation.scope, "src/main.rs");
        assert_eq!(
            observation.summary,
            "read src/main.rs lines 3:5 (truncated)"
        );
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), 1);
        assert_eq!(
            observation.evidence[0].reason,
            EVIDENCE_REASON_REQUESTED_FILE_CONTENT
        );
        assert_eq!(
            observation.next_observations,
            read_followup_observations("src/main.rs")
        );

        assert_eq!(
            read_followup_observations("path with spaces.txt"),
            vec![
                "workspace search 'path with spaces.txt'",
                "workspace diff --summary"
            ]
        );
    }

    #[test]
    fn observed_read_preserves_path_range_and_truncation() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::create_dir(temp.path().join("src")).expect("src directory should be created");
        let path = temp.path().join("src/main.rs");
        fs::write(&path, "one\ntwo\nthree\n").expect("source file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let read = observed_read(&workspace, &path, Some((2, 3))).expect("file should be read");

        assert_eq!(read.data.path, "src/main.rs");
        assert_eq!(read.data.lines.as_deref(), Some("2:3"));
        assert_eq!(read.data.content, "two\nthree");
        assert!(!read.content_truncated);
    }

    #[test]
    fn read_data_helpers_preserve_requested_content() {
        assert_eq!(read_line_label(Some((4, 9))).as_deref(), Some("4:9"));
        assert!(read_line_label(None).is_none());

        let data = read_data(
            "src/main.rs".to_string(),
            Some("4:9".to_string()),
            "fn main() {}\n".to_string(),
        );

        assert_eq!(data.path, "src/main.rs");
        assert_eq!(data.lines.as_deref(), Some("4:9"));
        assert_eq!(data.content, "fn main() {}\n");
    }

    #[test]
    fn observed_read_args_resolves_path_and_line_range() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::create_dir(temp.path().join("src")).expect("src directory should be created");
        fs::write(temp.path().join("src/main.rs"), "one\ntwo\nthree\n")
            .expect("source file should be written");
        let workspace = Workspace {
            root: temp
                .path()
                .canonicalize()
                .expect("temp path should resolve"),
            is_git_repo: false,
        };
        let args = ReadArgs {
            json: true,
            lines: Some("1:2".to_string()),
            path: PathBuf::from("src/main.rs"),
        };

        let read = observed_read_args(&workspace, &args).expect("read data should be observed");

        assert_eq!(read.data.path, "src/main.rs");
        assert_eq!(read.data.lines.as_deref(), Some("1:2"));
        assert_eq!(read.data.content, "one\ntwo");
        assert!(!read.content_truncated);
    }

    #[test]
    fn static_followup_helpers_report_expected_commands() {
        assert_eq!(
            status_next_observations(),
            vec![
                "workspace map",
                "workspace diff --summary",
                "workspace index status",
                "workspace log"
            ]
        );
        assert_eq!(
            index_status_next_observations(),
            vec![
                "workspace index cochange",
                "workspace related <file> --by cochange --use-index",
                "workspace impact --diff --by cochange --use-index"
            ]
        );
        assert_eq!(
            index_cochange_next_observations(),
            vec![
                "workspace related <file> --by cochange --use-index",
                "workspace impact --diff --by cochange --use-index"
            ]
        );
        assert_eq!(
            patch_followup_observations("tx-1"),
            vec!["workspace diff --summary", "workspace rollback tx-1"]
        );
        assert_eq!(
            run_followup_observations(),
            vec!["workspace status", "workspace diff --summary"]
        );
        assert_eq!(log_followup_observations(), vec!["workspace status"]);
        assert_eq!(
            rollback_followup_observations(),
            vec!["workspace diff --summary"]
        );
    }

    #[test]
    fn observation_kind_constants_match_json_contract() {
        assert_eq!(
            [
                WORKSPACE_MAP_KIND,
                WORKSPACE_STATUS_KIND,
                WORKSPACE_SEARCH_KIND,
                WORKSPACE_INDEX_STATUS_KIND,
                WORKSPACE_INDEX_COCHANGE_KIND,
                WORKSPACE_RELATED_KIND,
                WORKSPACE_IMPACT_KIND,
                WORKSPACE_READ_KIND,
                WORKSPACE_DIFF_KIND,
                WORKSPACE_PATCH_KIND,
                WORKSPACE_RUN_KIND,
                WORKSPACE_LOG_KIND,
                WORKSPACE_ROLLBACK_KIND,
            ],
            [
                "workspace_map",
                "workspace_status",
                "workspace_search",
                "workspace_index_status",
                "workspace_index_cochange",
                "workspace_related",
                "workspace_impact",
                "workspace_read",
                "workspace_diff",
                "workspace_patch",
                "workspace_run",
                "workspace_log",
                "workspace_rollback",
            ]
        );
    }

    #[test]
    fn evidence_reason_constants_match_json_contract() {
        assert_eq!(
            [
                EVIDENCE_REASON_GIT_DIFF_CHANGED_FILE,
                EVIDENCE_REASON_PATCH_FILE_TARGET,
                EVIDENCE_REASON_ROLLBACK_TARGET,
                EVIDENCE_REASON_TEXT_MATCH,
                EVIDENCE_REASON_REQUESTED_FILE_CONTENT,
                IMPORTANT_REASON_CONFIGURATION_OR_PACKAGE_MANIFEST,
                IMPORTANT_REASON_LIKELY_ENTRYPOINT,
                IMPORTANT_REASON_PRIMARY_PROJECT_DOCUMENTATION,
                IMPORTANT_REASON_NO_LANGUAGE_SIGNALS,
            ],
            [
                "git diff changed file",
                "patch file target",
                "rollback target",
                "text match",
                "requested file content",
                "configuration or package manifest",
                "likely entrypoint",
                "primary project documentation",
                "no language signals detected yet",
            ]
        );
    }

    #[test]
    fn summary_label_constants_match_human_contract() {
        assert_eq!(SUMMARY_NOT_GIT_REPOSITORY, "not a git repository");
        assert_eq!(OUTPUT_TRUNCATED_MARKER, "\n[output truncated]\n");
        assert_eq!(INLINE_TRUNCATED_MARKER, " [truncated]");
        assert_eq!(SUMMARY_NOTE_MAP_TRUNCATED, " (map truncated)");
        assert_eq!(SUMMARY_NOTE_STATUS_TRUNCATED, " (status truncated)");
        assert_eq!(SUMMARY_NOTE_SEED_FILES_TRUNCATED, " (seed files truncated)");
        assert_eq!(SUMMARY_NOTE_FILES_TRUNCATED, " (files truncated)");
        assert_eq!(
            SUMMARY_NOTE_SUMMARY_AND_PATCH_TRUNCATED,
            " (summary and patch truncated)"
        );
        assert_eq!(SUMMARY_NOTE_SUMMARY_TRUNCATED, " (summary truncated)");
        assert_eq!(SUMMARY_NOTE_PATCH_TRUNCATED, " (patch truncated)");
        assert_eq!(SUMMARY_NOTE_OUTPUT_TRUNCATED, " (output truncated)");
        assert_eq!(SUMMARY_NOTE_READ_TRUNCATED, " (truncated)");
        assert_eq!(
            SUMMARY_NOTE_OPERATION_LOG_UNREADABLE,
            ", operation log unreadable"
        );
        assert_eq!(
            SUMMARY_NOTE_RECENT_OPERATIONS_TRUNCATED,
            ", recent operations truncated"
        );
    }

    #[test]
    fn log_label_constants_match_operation_log_contract() {
        assert_eq!(
            [LOG_KIND_OBSERVE, LOG_KIND_CHANGE, LOG_KIND_VERIFY],
            ["observe", "change", "verify"]
        );
        assert_eq!(
            [
                LOG_OP_MAP,
                LOG_OP_STATUS,
                LOG_OP_SEARCH,
                LOG_OP_INDEX_STATUS,
                LOG_OP_INDEX_COCHANGE,
                LOG_OP_RELATED,
                LOG_OP_IMPACT,
                LOG_OP_READ,
                LOG_OP_DIFF,
                LOG_OP_PATCH,
                LOG_OP_RUN,
                LOG_OP_ROLLBACK,
            ],
            [
                "map",
                "status",
                "search",
                "index status",
                "index cochange",
                "related",
                "impact",
                "read",
                "diff",
                "patch",
                "run",
                "rollback",
            ]
        );
    }

    #[test]
    fn impact_source_constant_matches_json_contract() {
        assert_eq!(IMPACT_SOURCE_DIFF, "diff");
    }

    #[test]
    fn relationship_label_constants_match_json_contract() {
        assert_eq!(RELATED_METHOD_COCHANGE, "cochange");
        assert_eq!([RANK_DIRECT, RANK_PAGERANK], ["direct", "pagerank"]);
        assert_eq!(
            [
                RELATIONSHIP_SOURCE_COCHANGE_INDEX,
                RELATIONSHIP_SOURCE_GIT_LOG,
                RELATIONSHIP_SOURCE_RELATED_CLI,
            ],
            ["cochange-index", "git-log", "related-cli"]
        );
        assert_eq!(RelatedMethod::Cochange.as_str(), RELATED_METHOD_COCHANGE);
        assert_eq!(RankingMethod::Direct.as_str(), RANK_DIRECT);
        assert_eq!(RankingMethod::Pagerank.as_str(), RANK_PAGERANK);
        assert_eq!(
            relationship_source(true),
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert_eq!(relationship_source(false), RELATIONSHIP_SOURCE_GIT_LOG);
        assert_eq!(
            relationship_source_for_options(false, RankingMethod::Direct),
            RELATIONSHIP_SOURCE_GIT_LOG
        );
        assert_eq!(
            relationship_source_for_options(true, RankingMethod::Direct),
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert_eq!(
            relationship_source_for_options(false, RankingMethod::Pagerank),
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert_eq!(
            related_cli_relationship_source("direct"),
            "related-cli:direct"
        );
        assert_eq!(
            related_cli_aggregate_relationship_source(RankingMethod::Direct),
            "related-cli:direct:aggregate"
        );
        assert_eq!(max_cochanged_commits([2, 5, 3]), 5);
        assert_eq!(max_cochanged_commits([]), 0);
    }

    #[test]
    fn index_status_constants_match_json_contract() {
        assert_eq!(
            [
                INDEX_STATUS_FRESH,
                INDEX_STATUS_STALE,
                INDEX_STATUS_MISSING,
                INDEX_STATUS_INVALID,
                INDEX_STATUS_NOT_GIT_REPO,
            ],
            ["fresh", "stale", "missing", "invalid", "not_git_repo"]
        );
    }

    #[test]
    fn non_repo_relationship_data_uses_requested_labels() {
        let related = related_data_for_non_repo(
            "src/main.rs",
            &RelatedMethod::Cochange,
            RankingMethod::Direct,
            false,
            300,
            40,
        );
        assert_eq!(related.target, "src/main.rs");
        assert_eq!(related.method, RELATED_METHOD_COCHANGE);
        assert_eq!(related.ranking, RANK_DIRECT);
        assert_eq!(related.relationship_source, RELATIONSHIP_SOURCE_GIT_LOG);
        assert!(!related.is_repo);
        assert_eq!(related.max_commits, 300);
        assert_eq!(related.max_files_per_commit, 40);
        assert!(related.related.is_empty());

        let impact = impact_data_for_non_repo(
            &RelatedMethod::Cochange,
            RankingMethod::Pagerank,
            false,
            500,
            80,
        );
        assert_eq!(impact.source, IMPACT_SOURCE_DIFF);
        assert_eq!(impact.method, RELATED_METHOD_COCHANGE);
        assert_eq!(impact.ranking, RANK_PAGERANK);
        assert_eq!(
            impact.relationship_source,
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert!(!impact.is_repo);
        assert_eq!(impact.seed_file_count, 0);
        assert_eq!(impact.max_commits, 500);
        assert_eq!(impact.max_files_per_commit, 80);
        assert!(impact.impacted.is_empty());
    }

    #[test]
    fn relationship_metadata_constructor_preserves_labels() {
        let metadata = RelationshipMetadata::new(
            &RelatedMethod::Cochange,
            RankingMethod::Pagerank,
            RELATIONSHIP_SOURCE_COCHANGE_INDEX,
            true,
        );

        assert_eq!(metadata.method, RELATED_METHOD_COCHANGE);
        assert_eq!(metadata.ranking, RANK_PAGERANK);
        assert_eq!(
            metadata.relationship_source,
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert!(metadata.is_repo);

        let (method, ranking, relationship_source, is_repo) = RelationshipMetadata::new(
            &RelatedMethod::Cochange,
            RankingMethod::Direct,
            RELATIONSHIP_SOURCE_GIT_LOG,
            false,
        )
        .into_parts();
        assert_eq!(method, RELATED_METHOD_COCHANGE);
        assert_eq!(ranking, RANK_DIRECT);
        assert_eq!(relationship_source, RELATIONSHIP_SOURCE_GIT_LOG);
        assert!(!is_repo);
    }

    #[test]
    fn relationship_stats_and_limits_constructors_preserve_fields() {
        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 0,
            head: None,
            max_commits: 500,
            max_files_per_commit: 80,
            commits_scanned: 12,
            commits_indexed: 10,
            ignored_large_commits: 2,
            file_commit_counts: BTreeMap::new(),
            edges: vec![],
        };

        let index_stats = RelationshipStats::from_cochange_index(&index, 4);
        assert_eq!(index_stats.commits_scanned, 12);
        assert_eq!(index_stats.commits_matched, 4);
        assert_eq!(index_stats.ignored_large_commits, 2);
        assert_eq!(index_stats.into_parts(), (12, 4, 2));

        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/b.rs".to_string()],
            },
        ];
        let git_log_stats = RelationshipStats::from_git_log(&commits, 1, 3);
        assert_eq!(git_log_stats.commits_scanned, 2);
        assert_eq!(git_log_stats.commits_matched, 1);
        assert_eq!(git_log_stats.ignored_large_commits, 3);

        let cli_stats = RelationshipStats::from_related_cli(3);
        assert_eq!(cli_stats.commits_scanned, 0);
        assert_eq!(cli_stats.commits_matched, 3);
        assert_eq!(cli_stats.ignored_large_commits, 0);

        let option_limits = RelationshipLimits::from_options(300, 40);
        assert_eq!(option_limits.max_commits, 300);
        assert_eq!(option_limits.max_files_per_commit, 40);

        let limits = RelationshipLimits::from_cochange_index(&index);
        assert_eq!(limits.max_commits, 500);
        assert_eq!(limits.max_files_per_commit, 80);
        assert_eq!(limits.into_parts(), (500, 80));
    }

    #[test]
    fn cochange_related_data_preserves_relationship_metadata() {
        let data = cochange_related_data(
            RelatedDataMetadata::cochange(
                "src/main.rs",
                RankingMethod::Pagerank,
                RELATIONSHIP_SOURCE_COCHANGE_INDEX,
                true,
            ),
            RelationshipStats::new(8, 3, 2),
            RelationshipLimits::new(500, 80),
            vec![RelatedFile {
                path: "tests/cli.rs".to_string(),
                score: 0.75,
                cochanged_commits: 3,
                weighted_cochanges: 1.25,
                sample_commits: vec!["abc123".to_string()],
            }],
        );

        assert_eq!(data.target, "src/main.rs");
        assert_eq!(data.method, RELATED_METHOD_COCHANGE);
        assert_eq!(data.ranking, RANK_PAGERANK);
        assert_eq!(data.relationship_source, RELATIONSHIP_SOURCE_COCHANGE_INDEX);
        assert!(data.is_repo);
        assert_eq!(data.commits_scanned, 8);
        assert_eq!(data.commits_matched, 3);
        assert_eq!(data.ignored_large_commits, 2);
        assert_eq!(data.max_commits, 500);
        assert_eq!(data.max_files_per_commit, 80);
        assert_eq!(data.related.len(), 1);
        assert_eq!(data.related[0].path, "tests/cli.rs");
    }

    #[test]
    fn cochange_impact_data_preserves_seed_and_relationship_metadata() {
        let seed_files = vec![
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/c.rs".to_string(),
        ];

        let data = cochange_impact_data(
            ImpactDataMetadata::cochange(RankingMethod::Direct, RELATIONSHIP_SOURCE_GIT_LOG, true),
            SeedFileSummary::from_seed_files(&seed_files, 2),
            RelationshipStats::new(8, 3, 1),
            RelationshipLimits::new(500, 80),
            vec![ImpactFile {
                path: "tests/cli.rs".to_string(),
                score: 0.75,
                cochanged_commits: 3,
                weighted_cochanges: 1.25,
                seed_files: vec!["src/a.rs".to_string()],
                sample_commits: vec!["abc123".to_string()],
            }],
        );

        assert_eq!(data.source, IMPACT_SOURCE_DIFF);
        assert_eq!(data.method, RELATED_METHOD_COCHANGE);
        assert_eq!(data.ranking, RANK_DIRECT);
        assert_eq!(data.relationship_source, RELATIONSHIP_SOURCE_GIT_LOG);
        assert!(data.is_repo);
        assert_eq!(data.seed_files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(data.seed_file_count, 3);
        assert_eq!(data.omitted_seed_files, 1);
        assert_eq!(data.commits_scanned, 8);
        assert_eq!(data.commits_matched, 3);
        assert_eq!(data.ignored_large_commits, 1);
        assert_eq!(data.max_commits, 500);
        assert_eq!(data.max_files_per_commit, 80);
        assert_eq!(data.impacted.len(), 1);
        assert_eq!(data.impacted[0].path, "tests/cli.rs");
    }

    #[test]
    fn non_repo_observed_relationship_helpers_use_requested_labels() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let related_args = RelatedArgs {
            json: true,
            by: RelatedMethod::Cochange,
            max_commits: 300,
            max_files_per_commit: 40,
            max_results: 7,
            rank: RankingMethod::Pagerank,
            use_index: false,
            path: PathBuf::from("src/main.rs"),
        };
        let related = observed_related(&workspace, "src/main.rs", &related_args)
            .expect("related data should be built");
        assert_eq!(related.target, "src/main.rs");
        assert_eq!(related.method, RELATED_METHOD_COCHANGE);
        assert_eq!(related.ranking, RANK_PAGERANK);
        assert_eq!(
            related.relationship_source,
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert!(!related.is_repo);
        assert_eq!(related.max_commits, 300);
        assert_eq!(related.max_files_per_commit, 40);
        assert!(related.related.is_empty());

        let observed_related = observed_related_args(&workspace, &related_args)
            .expect("related args should be observed");
        assert_eq!(observed_related.target, "src/main.rs");
        assert_eq!(observed_related.data.target, "src/main.rs");
        assert_eq!(observed_related.data.ranking, RANK_PAGERANK);

        let impact_args = ImpactArgs {
            json: true,
            diff: true,
            by: RelatedMethod::Cochange,
            max_commits: 500,
            max_files_per_commit: 80,
            max_results: 9,
            rank: RankingMethod::Direct,
            use_index: true,
        };
        let impact =
            observed_impact_args(&workspace, &impact_args).expect("impact data should be built");
        assert_eq!(impact.source, IMPACT_SOURCE_DIFF);
        assert_eq!(impact.method, RELATED_METHOD_COCHANGE);
        assert_eq!(impact.ranking, RANK_DIRECT);
        assert_eq!(
            impact.relationship_source,
            RELATIONSHIP_SOURCE_COCHANGE_INDEX
        );
        assert!(!impact.is_repo);
        assert_eq!(impact.max_commits, 500);
        assert_eq!(impact.max_files_per_commit, 80);
        assert!(impact.impacted.is_empty());

        let invalid_impact_args = ImpactArgs {
            json: true,
            diff: false,
            by: RelatedMethod::Cochange,
            max_commits: 500,
            max_files_per_commit: 80,
            max_results: 9,
            rank: RankingMethod::Direct,
            use_index: true,
        };
        let error = match observed_impact_args(&workspace, &invalid_impact_args) {
            Ok(_) => panic!("impact without --diff should fail"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "workspace impact currently supports only --diff as its source"
        );
    }

    #[test]
    fn observation_truncation_helpers_report_data_limits() {
        let search = SearchData {
            query: "needle".to_string(),
            total_matches: 2,
            truncated_match_texts: 0,
            matches: vec![SearchMatch {
                path: "a.txt".to_string(),
                line: 1,
                column: 1,
                text: "needle".to_string(),
            }],
        };
        assert!(search_truncated(&search));
        assert!(search_results_omitted(&search));
        assert!(!search_match_texts_truncated(&search));

        let search = SearchData {
            total_matches: 1,
            truncated_match_texts: 1,
            ..search
        };
        assert!(search_truncated(&search));
        assert!(!search_results_omitted(&search));
        assert!(search_match_texts_truncated(&search));

        let search = SearchData {
            total_matches: 1,
            truncated_match_texts: 0,
            ..search
        };
        assert!(!search_truncated(&search));
        assert!(!search_results_omitted(&search));
        assert!(!search_match_texts_truncated(&search));

        let impact = ImpactData {
            source: IMPACT_SOURCE_DIFF.to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_DIRECT.to_string(),
            relationship_source: "git history".to_string(),
            is_repo: true,
            seed_files: vec![],
            seed_file_count: 2,
            omitted_seed_files: 1,
            commits_scanned: 0,
            commits_matched: 0,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            impacted: vec![],
        };
        assert!(impact_truncated(&impact));
        assert!(impact_seed_files_omitted(&impact));

        let impact = ImpactData {
            omitted_seed_files: 0,
            ..impact
        };
        assert!(!impact_truncated(&impact));
        assert!(!impact_seed_files_omitted(&impact));

        let diff = DiffData {
            is_repo: true,
            summary: String::new(),
            file_count: 1,
            files: vec!["a.txt".to_string()],
            omitted_files: 1,
            patch: None,
        };
        assert!(diff_truncated(&diff, false, false));
        assert!(diff_files_omitted(&diff));
        assert!(diff_truncated(&diff, true, false));

        let diff = DiffData {
            omitted_files: 0,
            ..diff
        };
        assert!(diff_truncated(&diff, false, true));
        assert!(!diff_truncated(&diff, false, false));
        assert!(!diff_files_omitted(&diff));

        let log = LogData {
            log_path: ".workspace/log.jsonl".to_string(),
            omitted_lines: 1,
            entries: vec![],
        };
        assert!(log_truncated(&log));
        assert!(log_lines_omitted(&log));

        let log = LogData {
            omitted_lines: 0,
            ..log
        };
        assert!(!log_truncated(&log));
        assert!(!log_lines_omitted(&log));
    }

    #[test]
    fn search_observation_helpers_are_bounded() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let matches = (0..(MAX_EVIDENCE_ITEMS + 1))
            .map(|index| SearchMatch {
                path: format!("file_{index:02}.txt"),
                line: index as u64 + 1,
                column: 1,
                text: "match".to_string(),
            })
            .collect::<Vec<_>>();

        let evidence = search_evidence(&matches);
        let next = search_next_observations(&matches);

        assert_eq!(evidence.len(), MAX_EVIDENCE_ITEMS);
        assert_eq!(evidence[0].path, "file_00.txt");
        assert_eq!(evidence[0].lines.as_deref(), Some("1"));
        assert_eq!(evidence[0].reason, EVIDENCE_REASON_TEXT_MATCH);
        assert_eq!(next.len(), MAX_NEXT_OBSERVATIONS);
        assert_eq!(next[0], "workspace read file_00.txt --lines 1:1");
        assert_eq!(next[4], "workspace read file_04.txt --lines 5:5");

        let data = search_data("needle", matches, MAX_EVIDENCE_ITEMS + 2, 1);
        let observation = search_observation(&workspace, data);
        assert_eq!(observation.kind, WORKSPACE_SEARCH_KIND);
        assert_eq!(
            observation.scope,
            temp.path().to_string_lossy().into_owned()
        );
        assert_eq!(
            observation.summary,
            format!(
                "{} match(es) for \"needle\", showing {}, truncated 1 match text(s)",
                MAX_EVIDENCE_ITEMS + 2,
                MAX_EVIDENCE_ITEMS + 1
            )
        );
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), MAX_EVIDENCE_ITEMS);
        assert_eq!(observation.evidence[0].reason, EVIDENCE_REASON_TEXT_MATCH);
        assert_eq!(observation.next_observations.len(), MAX_NEXT_OBSERVATIONS);
    }

    #[test]
    fn observed_search_preserves_total_matches_with_limited_results() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::write(temp.path().join("a.txt"), "needle one\nneedle two\n")
            .expect("file should be written");
        fs::write(temp.path().join("b.txt"), "other\n").expect("file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let args = SearchArgs {
            json: true,
            max_results: 1,
            query: "needle".to_string(),
        };

        let data = observed_search(&workspace, &args).expect("search data should be observed");

        assert_eq!(data.query, "needle");
        assert_eq!(data.total_matches, 2);
        assert_eq!(data.matches.len(), 1);
        assert_eq!(data.matches[0].path, "a.txt");
        assert_eq!(data.matches[0].line, 1);
        assert_eq!(data.truncated_match_texts, 0);
    }

    fn test_git_summary(is_repo: bool) -> GitSummary {
        GitSummary {
            is_repo,
            branch: Some("main".to_string()),
            dirty_file_count: 2,
            untracked_file_count: 1,
            dirty_files: vec![],
            untracked_files: vec![],
            omitted_dirty_files: 0,
            omitted_untracked_files: 0,
        }
    }

    fn test_index_status_data(status: &str) -> IndexStatusData {
        IndexStatusData {
            is_repo: status != INDEX_STATUS_NOT_GIT_REPO,
            path: ".workspace/index/cochange.json".to_string(),
            exists: status != INDEX_STATUS_MISSING,
            readable: status != INDEX_STATUS_INVALID,
            status: status.to_string(),
            fresh: status == INDEX_STATUS_FRESH,
            current_head: Some("abc123".to_string()),
            index_head: Some("abc123".to_string()),
            generated_at_unix_ms: Some(1),
            max_commits: Some(500),
            max_files_per_commit: Some(100),
            commits_scanned: Some(5),
            commits_indexed: Some(4),
            ignored_large_commits: Some(1),
            file_count: Some(3),
            edge_count: Some(2),
            error: None,
        }
    }

    fn test_status_data() -> StatusData {
        StatusData {
            root: "/repo".to_string(),
            git: test_git_summary(true),
            index_status: test_index_status_data(INDEX_STATUS_FRESH),
            recent_operations: vec![],
            recent_operations_omitted: 0,
            recent_operations_error: None,
        }
    }

    fn test_workspace_map() -> WorkspaceMap {
        WorkspaceMap {
            root: "/repo".to_string(),
            git: test_git_summary(true),
            stack: StackSummary {
                languages: vec!["Rust".to_string(), "TypeScript".to_string()],
                package_managers: vec![],
                frameworks: vec![],
            },
            structure: StructureSummary {
                directories: vec![],
                entrypoints: vec![],
                tests: vec![],
                configs: vec![],
                docs: vec![],
            },
            commands: BTreeMap::new(),
            stats: WorkspaceStats {
                file_count: 3,
                directory_count: 1,
                large_files: vec![],
            },
            important_files: vec![],
            recent_files: vec![],
            omitted: MapOmittedCounts::default(),
        }
    }

    #[test]
    fn map_summary_reports_languages_and_truncation() {
        let mut map = test_workspace_map();
        assert!(!map_truncated(&map));
        assert_eq!(
            map_summary(&map, map_truncated(&map)),
            "3 file(s), languages: Rust, TypeScript"
        );

        map.omitted.docs = 1;
        assert!(map_truncated(&map));
        assert_eq!(
            map_summary(&map, map_truncated(&map)),
            "3 file(s), languages: Rust, TypeScript (map truncated)"
        );

        map.omitted.docs = 0;
        map.git.omitted_dirty_files = 1;
        assert!(map_truncated(&map));
        map.stack.languages.clear();
        assert_eq!(
            map_summary(&map, map_truncated(&map)),
            "3 file(s), languages: none (map truncated)"
        );
    }

    #[test]
    fn map_observation_helper_preserves_contract() {
        let mut map = test_workspace_map();
        map.structure.docs = vec!["README.md".to_string()];
        map.structure.entrypoints = vec!["src/main.rs".to_string()];
        map.important_files = vec![
            ImportantFile {
                path: "README.md".to_string(),
                reason: IMPORTANT_REASON_PRIMARY_PROJECT_DOCUMENTATION.to_string(),
            },
            ImportantFile {
                path: "src/main.rs".to_string(),
                reason: IMPORTANT_REASON_LIKELY_ENTRYPOINT.to_string(),
            },
        ];
        map.commands
            .insert("test".to_string(), "cargo test".to_string());

        let observation = map_observation(map);
        assert_eq!(observation.kind, WORKSPACE_MAP_KIND);
        assert_eq!(observation.scope, "/repo");
        assert_eq!(
            observation.summary,
            "3 file(s), languages: Rust, TypeScript"
        );
        assert!(!observation.truncated);
        assert_eq!(observation.evidence.len(), 2);
        assert_eq!(
            observation.evidence[0].reason,
            IMPORTANT_REASON_PRIMARY_PROJECT_DOCUMENTATION
        );
        assert_eq!(
            observation.next_observations,
            vec![
                "workspace read README.md",
                "workspace read src/main.rs",
                WORKSPACE_DIFF_SUMMARY_COMMAND,
                WORKSPACE_INDEX_STATUS_COMMAND,
                WORKSPACE_INDEX_COCHANGE_COMMAND,
                WORKSPACE_IMPACT_COCHANGE_COMMAND,
                "workspace related src/main.rs --by cochange",
                "workspace run 'cargo test'",
            ]
        );
    }

    #[test]
    fn observed_map_respects_hidden_file_filter() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::create_dir(temp.path().join("src")).expect("src directory should be created");
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n")
            .expect("source file should be written");
        fs::write(temp.path().join(".hidden.txt"), "hidden\n")
            .expect("hidden file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let hidden_excluded = observed_map(
            &workspace,
            &MapArgs {
                json: true,
                depth: 2,
                include_hidden: false,
            },
        )
        .expect("map should be observed");
        let hidden_included = observed_map(
            &workspace,
            &MapArgs {
                json: true,
                depth: 2,
                include_hidden: true,
            },
        )
        .expect("map should include hidden files");

        assert_eq!(hidden_excluded.stats.file_count, 1);
        assert_eq!(hidden_included.stats.file_count, 2);
        assert!(
            !hidden_excluded
                .recent_files
                .iter()
                .any(|path| path == ".hidden.txt")
        );
    }

    #[test]
    fn status_summary_reports_log_notes_and_truncation() {
        let mut data = test_status_data();
        assert!(!status_truncated(&data));
        assert!(!status_recent_operations_omitted(&data));
        assert_eq!(
            status_summary(&data, status_truncated(&data)),
            "branch main, 2 dirty file(s), 1 untracked file(s), index fresh"
        );

        data.recent_operations_omitted = 3;
        assert!(status_truncated(&data));
        assert!(status_recent_operations_omitted(&data));
        assert_eq!(
            status_summary(&data, status_truncated(&data)),
            "branch main, 2 dirty file(s), 1 untracked file(s), index fresh, recent operations truncated (status truncated)"
        );

        data.recent_operations_omitted = 0;
        data.recent_operations_error = Some("bad log".to_string());
        assert!(!status_truncated(&data));
        assert!(!status_recent_operations_omitted(&data));
        assert_eq!(
            status_summary(&data, status_truncated(&data)),
            "branch main, 2 dirty file(s), 1 untracked file(s), index fresh, operation log unreadable"
        );

        data.git.is_repo = false;
        data.recent_operations_error = None;
        data.recent_operations_omitted = 2;
        assert!(status_recent_operations_omitted(&data));
        assert_eq!(
            status_summary(&data, status_truncated(&data)),
            "not a git repository (status truncated)"
        );
    }

    #[test]
    fn status_data_helpers_preserve_recent_operation_state() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let recent = StatusRecentOperations {
            entries: vec![LogEntry {
                id: "op-1".to_string(),
                timestamp_unix_ms: 1,
                kind: LOG_KIND_OBSERVE.to_string(),
                op: LOG_OP_STATUS.to_string(),
                scope: ".".to_string(),
                summary: "entry".to_string(),
                transaction_id: None,
            }],
            omitted_lines: 2,
            error: None,
        };

        let data = status_data(
            &workspace,
            test_git_summary(true),
            test_index_status_data(INDEX_STATUS_FRESH),
            recent,
        );
        assert_eq!(data.root, temp.path().to_string_lossy().into_owned());
        assert_eq!(data.recent_operations.len(), 1);
        assert_eq!(data.recent_operations_omitted, 2);
        assert!(data.recent_operations_error.is_none());

        fs::create_dir_all(temp.path().join(LOG_FILE)).expect("log path directory should exist");
        let recent = read_status_recent_operations(&workspace, 10);
        assert!(recent.entries.is_empty());
        assert_eq!(recent.omitted_lines, 0);
        assert!(
            recent
                .error
                .as_deref()
                .is_some_and(|error| error.contains("not a file"))
        );
    }

    #[test]
    fn observed_status_reports_non_repo_index_state() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let data = observed_status(&workspace).expect("status data should be observed");

        assert_eq!(data.root, temp.path().to_string_lossy().into_owned());
        assert!(!data.git.is_repo);
        assert_eq!(data.git.dirty_file_count, 0);
        assert_eq!(data.index_status.status, INDEX_STATUS_NOT_GIT_REPO);
        assert!(!data.index_status.is_repo);
        assert!(data.recent_operations.is_empty());
        assert_eq!(data.recent_operations_omitted, 0);
    }

    #[test]
    fn status_observation_helper_preserves_contract() {
        let mut data = test_status_data();
        data.recent_operations_omitted = 1;

        let observation = status_observation(data);
        assert_eq!(observation.kind, WORKSPACE_STATUS_KIND);
        assert_eq!(observation.scope, "/repo");
        assert_eq!(
            observation.summary,
            "branch main, 2 dirty file(s), 1 untracked file(s), index fresh, recent operations truncated (status truncated)"
        );
        assert!(observation.truncated);
        assert!(observation.evidence.is_empty());
        assert_eq!(observation.next_observations, status_next_observations());
    }

    #[test]
    fn index_status_summary_reports_known_statuses() {
        for (status, expected) in [
            (INDEX_STATUS_FRESH, "co-change index is fresh"),
            (INDEX_STATUS_STALE, "co-change index is stale"),
            (INDEX_STATUS_MISSING, "co-change index is missing"),
            (INDEX_STATUS_INVALID, "co-change index is invalid"),
            (INDEX_STATUS_NOT_GIT_REPO, SUMMARY_NOT_GIT_REPOSITORY),
            ("custom", "custom"),
        ] {
            assert_eq!(
                index_status_summary(&test_index_status_data(status)),
                expected
            );
        }
    }

    #[test]
    fn index_status_observation_helper_preserves_contract() {
        let data = test_index_status_data(INDEX_STATUS_STALE);

        let observation = index_status_observation(data);
        assert_eq!(observation.kind, WORKSPACE_INDEX_STATUS_KIND);
        assert_eq!(observation.scope, ".workspace/index/cochange.json");
        assert_eq!(observation.summary, "co-change index is stale");
        assert!(!observation.truncated);
        assert!(observation.evidence.is_empty());
        assert_eq!(
            observation.next_observations,
            index_status_next_observations()
        );
    }

    #[test]
    fn observed_index_status_reports_non_repo_state() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let data = observed_index_status(&workspace);

        assert_eq!(data.path, COCHANGE_INDEX_FILE);
        assert!(!data.is_repo);
        assert_eq!(data.status, INDEX_STATUS_NOT_GIT_REPO);
        assert!(!data.exists);
        assert!(!data.fresh);
    }

    #[test]
    fn readable_index_status_preserves_index_metadata() {
        assert_eq!(index_freshness_status(true), INDEX_STATUS_FRESH);
        assert_eq!(index_freshness_status(false), INDEX_STATUS_STALE);

        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 123,
            head: Some("abc123".to_string()),
            max_commits: 500,
            max_files_per_commit: 100,
            commits_scanned: 5,
            commits_indexed: 4,
            ignored_large_commits: 1,
            file_commit_counts: BTreeMap::from([
                ("src/main.rs".to_string(), 2),
                ("tests/cli.rs".to_string(), 1),
            ]),
            edges: vec![CochangeEdge {
                a: "src/main.rs".to_string(),
                b: "tests/cli.rs".to_string(),
                cochanged_commits: 1,
                weighted_cochanges: 1.0,
                sample_commits: vec!["abc123".to_string()],
            }],
        };

        let data = readable_index_status(
            COCHANGE_INDEX_FILE.to_string(),
            Some("abc123".to_string()),
            index,
        );

        assert!(data.is_repo);
        assert_eq!(data.path, COCHANGE_INDEX_FILE);
        assert!(data.exists);
        assert!(data.readable);
        assert_eq!(data.status, INDEX_STATUS_FRESH);
        assert!(data.fresh);
        assert_eq!(data.current_head.as_deref(), Some("abc123"));
        assert_eq!(data.index_head.as_deref(), Some("abc123"));
        assert_eq!(data.generated_at_unix_ms, Some(123));
        assert_eq!(data.max_commits, Some(500));
        assert_eq!(data.max_files_per_commit, Some(100));
        assert_eq!(data.commits_scanned, Some(5));
        assert_eq!(data.commits_indexed, Some(4));
        assert_eq!(data.ignored_large_commits, Some(1));
        assert_eq!(data.file_count, Some(2));
        assert_eq!(data.edge_count, Some(1));
        assert!(data.error.is_none());
    }

    #[test]
    fn index_cochange_summary_reports_edges_and_commits() {
        let data = IndexCochangeData {
            path: ".workspace/index/cochange.json".to_string(),
            version: 1,
            generated_at_unix_ms: 1,
            head: Some("abc123".to_string()),
            max_commits: 500,
            max_files_per_commit: 100,
            commits_scanned: 5,
            commits_indexed: 4,
            ignored_large_commits: 1,
            file_count: 3,
            edge_count: 2,
        };

        assert_eq!(
            index_cochange_summary(&data),
            "indexed 2 co-change edge(s) from 4 commit(s)"
        );
    }

    #[test]
    fn index_cochange_helpers_preserve_contract() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let index_path = temp.path().join(COCHANGE_INDEX_FILE);
        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 123,
            head: Some("abc123".to_string()),
            max_commits: 500,
            max_files_per_commit: 100,
            commits_scanned: 5,
            commits_indexed: 4,
            ignored_large_commits: 1,
            file_commit_counts: BTreeMap::from([
                ("src/main.rs".to_string(), 2),
                ("tests/cli.rs".to_string(), 1),
            ]),
            edges: vec![CochangeEdge {
                a: "src/main.rs".to_string(),
                b: "tests/cli.rs".to_string(),
                cochanged_commits: 1,
                weighted_cochanges: 1.0,
                sample_commits: vec!["abc123".to_string()],
            }],
        };

        let data = index_cochange_data(&workspace, &index_path, &index);
        assert_eq!(data.path, COCHANGE_INDEX_FILE);
        assert_eq!(data.file_count, 2);
        assert_eq!(data.edge_count, 1);

        let observation = index_cochange_observation(data);
        assert_eq!(observation.kind, WORKSPACE_INDEX_COCHANGE_KIND);
        assert_eq!(observation.scope, COCHANGE_INDEX_FILE);
        assert_eq!(
            observation.summary,
            "indexed 1 co-change edge(s) from 4 commit(s)"
        );
        assert!(!observation.truncated);
        assert!(observation.evidence.is_empty());
        assert_eq!(
            observation.next_observations,
            index_cochange_next_observations()
        );

        let saved_path =
            write_workspace_cochange_index(&workspace, &index).expect("index should be written");
        assert_eq!(saved_path, temp.path().join(COCHANGE_INDEX_FILE));
        let loaded =
            read_cochange_index_from_path(&saved_path).expect("written index should be readable");
        assert_eq!(loaded.max_commits, 500);
        assert_eq!(loaded.edges.len(), 1);
    }

    #[test]
    fn observed_index_cochange_requires_git_repo() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let args = IndexCochangeArgs {
            json: true,
            max_commits: 1000,
            max_files_per_commit: 40,
        };

        let error = match observed_index_cochange(&workspace, &args) {
            Ok(_) => panic!("non-repo workspace should not build a co-change index"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "workspace index cochange requires a git repository"
        );
        assert!(!workspace.log_path().exists());
    }

    #[test]
    fn related_summary_reports_repo_state() {
        let data = RelatedData {
            target: "src/main.rs".to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_DIRECT.to_string(),
            relationship_source: "git history".to_string(),
            is_repo: true,
            commits_scanned: 3,
            commits_matched: 1,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            related: vec![RelatedFile {
                path: "tests/cli.rs".to_string(),
                score: 1.0,
                cochanged_commits: 1,
                weighted_cochanges: 1.0,
                sample_commits: vec!["abc123".to_string()],
            }],
        };
        assert_eq!(
            related_summary(&data),
            "1 related file(s) for src/main.rs using cochange history"
        );

        let data = RelatedData {
            is_repo: false,
            related: vec![],
            ..data
        };
        assert_eq!(related_summary(&data), SUMMARY_NOT_GIT_REPOSITORY);
    }

    #[test]
    fn impact_summary_reports_seed_truncation() {
        let data = ImpactData {
            source: IMPACT_SOURCE_DIFF.to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_DIRECT.to_string(),
            relationship_source: "git history".to_string(),
            is_repo: true,
            seed_files: vec!["src/main.rs".to_string()],
            seed_file_count: 3,
            omitted_seed_files: 2,
            commits_scanned: 5,
            commits_matched: 2,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            impacted: vec![ImpactFile {
                path: "tests/cli.rs".to_string(),
                score: 1.0,
                cochanged_commits: 1,
                weighted_cochanges: 1.0,
                seed_files: vec!["src/main.rs".to_string()],
                sample_commits: vec!["abc123".to_string()],
            }],
        };
        assert_eq!(
            impact_summary(&data),
            "1 impacted file(s) from 3 seed file(s) using cochange history (seed files truncated)"
        );
        assert!(impact_seed_files_omitted(&data));

        let data = ImpactData {
            is_repo: false,
            impacted: vec![],
            omitted_seed_files: 0,
            ..data
        };
        assert_eq!(impact_summary(&data), SUMMARY_NOT_GIT_REPOSITORY);
        assert!(!impact_seed_files_omitted(&data));
    }

    #[test]
    fn relationship_observation_helpers_preserve_contract() {
        assert_eq!(
            relationship_scan_summary(5, 2, 1),
            "  scanned: 5 commit(s), matched: 2, ignored broad commits: 1"
        );

        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::create_dir(temp.path().join("src")).expect("src directory should be created");
        fs::write(temp.path().join("src/b.rs"), "b\n").expect("related file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };

        let related = RelatedData {
            target: "src/a.rs".to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_DIRECT.to_string(),
            relationship_source: RELATIONSHIP_SOURCE_GIT_LOG.to_string(),
            is_repo: true,
            commits_scanned: 5,
            commits_matched: 2,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            related: vec![
                RelatedFile {
                    path: "src/b.rs".to_string(),
                    score: 1.0,
                    cochanged_commits: 1,
                    weighted_cochanges: 1.0,
                    sample_commits: vec!["abc123".to_string()],
                },
                RelatedFile {
                    path: "src/missing.rs".to_string(),
                    score: 0.5,
                    cochanged_commits: 1,
                    weighted_cochanges: 0.5,
                    sample_commits: vec!["def456".to_string()],
                },
            ],
        };

        let observation = related_observation(&workspace, "src/a.rs", related);
        assert_eq!(observation.kind, WORKSPACE_RELATED_KIND);
        assert_eq!(observation.scope, "src/a.rs");
        assert_eq!(
            observation.summary,
            "2 related file(s) for src/a.rs using cochange history"
        );
        assert!(!observation.truncated);
        assert_eq!(observation.evidence.len(), 2);
        assert_eq!(
            observation.evidence[0].reason,
            "changed with src/a.rs in 1 commit(s); samples: abc123"
        );
        assert_eq!(
            observation.next_observations,
            vec!["workspace read src/b.rs"]
        );

        let impact = ImpactData {
            source: IMPACT_SOURCE_DIFF.to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_DIRECT.to_string(),
            relationship_source: RELATIONSHIP_SOURCE_GIT_LOG.to_string(),
            is_repo: true,
            seed_files: vec!["src/a.rs".to_string()],
            seed_file_count: 3,
            omitted_seed_files: 1,
            commits_scanned: 5,
            commits_matched: 2,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            impacted: vec![
                ImpactFile {
                    path: "src/b.rs".to_string(),
                    score: 1.0,
                    cochanged_commits: 1,
                    weighted_cochanges: 1.0,
                    seed_files: vec!["src/a.rs".to_string()],
                    sample_commits: vec!["abc123".to_string()],
                },
                ImpactFile {
                    path: "src/missing.rs".to_string(),
                    score: 0.5,
                    cochanged_commits: 1,
                    weighted_cochanges: 0.5,
                    seed_files: vec!["src/a.rs".to_string()],
                    sample_commits: vec!["def456".to_string()],
                },
            ],
        };

        let observation = impact_observation(&workspace, impact);
        assert_eq!(observation.kind, WORKSPACE_IMPACT_KIND);
        assert_eq!(observation.scope, IMPACT_SOURCE_DIFF);
        assert_eq!(
            observation.summary,
            "2 impacted file(s) from 3 seed file(s) using cochange history (seed files truncated)"
        );
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), 2);
        assert_eq!(
            observation.evidence[0].reason,
            "changed with seed file(s) src/a.rs in 1 commit(s); samples: abc123"
        );
        assert_eq!(
            observation.next_observations,
            vec!["workspace read src/b.rs"]
        );
    }

    #[test]
    fn relationship_evidence_reason_helpers_preserve_contract() {
        assert!(is_pagerank_only_hit(RANK_PAGERANK, 0));
        assert!(!is_pagerank_only_hit(RANK_PAGERANK, 1));
        assert!(!is_pagerank_only_hit(RANK_DIRECT, 0));

        let related = RelatedData {
            target: "src/a.rs".to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_PAGERANK.to_string(),
            relationship_source: RELATIONSHIP_SOURCE_COCHANGE_INDEX.to_string(),
            is_repo: true,
            commits_scanned: 5,
            commits_matched: 2,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            related: vec![],
        };
        let direct_related = RelatedFile {
            path: "src/b.rs".to_string(),
            score: 0.75,
            cochanged_commits: 2,
            weighted_cochanges: 1.5,
            sample_commits: vec!["abc123".to_string()],
        };
        let pagerank_related = RelatedFile {
            path: "src/c.rs".to_string(),
            score: 0.12345,
            cochanged_commits: 0,
            weighted_cochanges: 0.0,
            sample_commits: vec![],
        };

        assert_eq!(
            related_evidence_reason(&related, &direct_related),
            "changed with src/a.rs in 2 commit(s); samples: abc123"
        );
        assert_eq!(
            direct_evidence_reason("src/a.rs", 2, &["abc123".to_string()]),
            "changed with src/a.rs in 2 commit(s); samples: abc123"
        );
        assert_eq!(
            seed_files_evidence_subject(&["src/a.rs".to_string(), "src/other.rs".to_string()]),
            "seed file(s) src/a.rs, src/other.rs"
        );
        assert_eq!(
            related_evidence_reason(&related, &pagerank_related),
            "reached from src/a.rs through the co-change graph; pagerank score 0.123"
        );
        assert_eq!(
            pagerank_evidence_reason("src/a.rs", 0.12345),
            "reached from src/a.rs through the co-change graph; pagerank score 0.123"
        );

        let impact = ImpactData {
            source: IMPACT_SOURCE_DIFF.to_string(),
            method: RELATED_METHOD_COCHANGE.to_string(),
            ranking: RANK_PAGERANK.to_string(),
            relationship_source: RELATIONSHIP_SOURCE_COCHANGE_INDEX.to_string(),
            is_repo: true,
            seed_files: vec!["src/a.rs".to_string()],
            seed_file_count: 1,
            omitted_seed_files: 0,
            commits_scanned: 5,
            commits_matched: 2,
            ignored_large_commits: 0,
            max_commits: 500,
            max_files_per_commit: 100,
            impacted: vec![],
        };
        let direct_impact = ImpactFile {
            path: "src/b.rs".to_string(),
            score: 0.75,
            cochanged_commits: 2,
            weighted_cochanges: 1.5,
            seed_files: vec!["src/a.rs".to_string(), "src/other.rs".to_string()],
            sample_commits: vec!["abc123".to_string()],
        };
        let pagerank_impact = ImpactFile {
            path: "src/c.rs".to_string(),
            score: 0.98765,
            cochanged_commits: 0,
            weighted_cochanges: 0.0,
            seed_files: vec!["src/a.rs".to_string()],
            sample_commits: vec![],
        };

        assert_eq!(
            impact_evidence_reason(&impact, &direct_impact),
            "changed with seed file(s) src/a.rs, src/other.rs in 2 commit(s); samples: abc123"
        );
        assert_eq!(
            impact_evidence_reason(&impact, &pagerank_impact),
            "reached from seed file(s) src/a.rs through the co-change graph; pagerank score 0.988"
        );
    }

    #[test]
    fn search_summary_reports_truncated_results_and_text() {
        let data = SearchData {
            query: "needle".to_string(),
            total_matches: 3,
            truncated_match_texts: 2,
            matches: vec![SearchMatch {
                path: "a.txt".to_string(),
                line: 1,
                column: 1,
                text: "needle".to_string(),
            }],
        };

        assert_eq!(
            search_summary(&data),
            "3 match(es) for \"needle\", showing 1, truncated 2 match text(s)"
        );
        assert!(search_results_omitted(&data));
        assert!(search_match_texts_truncated(&data));

        let data = SearchData {
            query: "needle".to_string(),
            total_matches: 1,
            truncated_match_texts: 0,
            matches: vec![SearchMatch {
                path: "a.txt".to_string(),
                line: 1,
                column: 1,
                text: "needle".to_string(),
            }],
        };

        assert_eq!(search_summary(&data), "1 match(es) for \"needle\"");
        assert!(!search_results_omitted(&data));
        assert!(!search_match_texts_truncated(&data));
    }

    #[test]
    fn diff_summary_reports_each_truncation_kind() {
        assert_eq!(
            diff_output_truncation_note(true, true),
            Some(" (summary and patch truncated)")
        );
        assert_eq!(
            diff_output_truncation_note(true, false),
            Some(" (summary truncated)")
        );
        assert_eq!(
            diff_output_truncation_note(false, true),
            Some(" (patch truncated)")
        );
        assert_eq!(diff_output_truncation_note(false, false), None);

        let data = DiffData {
            is_repo: true,
            summary: "ignored for repositories".to_string(),
            file_count: 3,
            files: vec![],
            omitted_files: 2,
            patch: None,
        };

        assert_eq!(
            diff_summary(&data, true, true),
            "3 changed file(s) (summary and patch truncated) (files truncated)"
        );
        assert!(diff_files_omitted(&data));
        assert_eq!(
            diff_summary(&data, true, false),
            "3 changed file(s) (summary truncated) (files truncated)"
        );
        assert_eq!(
            diff_summary(&data, false, true),
            "3 changed file(s) (patch truncated) (files truncated)"
        );

        let data = DiffData {
            is_repo: false,
            summary: SUMMARY_NOT_GIT_REPOSITORY.to_string(),
            file_count: 0,
            files: vec![],
            omitted_files: 0,
            patch: None,
        };

        assert_eq!(
            diff_summary(&data, false, false),
            SUMMARY_NOT_GIT_REPOSITORY
        );
        assert!(!diff_files_omitted(&data));
    }

    #[test]
    fn diff_observation_helper_preserves_contract() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::create_dir(temp.path().join("src")).expect("src directory should be created");
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n")
            .expect("source file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let data = DiffData {
            is_repo: true,
            summary: "ignored for repositories".to_string(),
            file_count: 2,
            files: vec!["src/main.rs".to_string(), "missing.rs".to_string()],
            omitted_files: 1,
            patch: None,
        };
        let diff = ObservedDiff {
            data,
            summary_truncated: false,
            patch_truncated: false,
        };

        let observation = diff_observation(&workspace, diff);
        assert_eq!(observation.kind, WORKSPACE_DIFF_KIND);
        assert_eq!(
            observation.scope,
            temp.path().to_string_lossy().into_owned()
        );
        assert_eq!(observation.summary, "2 changed file(s) (files truncated)");
        assert!(observation.truncated);
        assert_eq!(observation.evidence.len(), 2);
        assert_eq!(
            observation.evidence[0].reason,
            EVIDENCE_REASON_GIT_DIFF_CHANGED_FILE
        );
        assert_eq!(
            observation.next_observations,
            vec!["workspace read src/main.rs"]
        );
    }

    #[test]
    fn diff_data_helper_preserves_changed_file_counts() {
        let data = diff_data(
            true,
            "summary".to_string(),
            BoundedFileList {
                files: vec!["src/main.rs".to_string()],
                total_files: 3,
                omitted_files: 2,
            },
            Some("patch".to_string()),
        );

        assert!(data.is_repo);
        assert_eq!(data.summary, "summary");
        assert_eq!(data.file_count, 3);
        assert_eq!(data.files, vec!["src/main.rs"]);
        assert_eq!(data.omitted_files, 2);
        assert_eq!(data.patch.as_deref(), Some("patch"));
    }

    #[test]
    fn non_repo_observed_diff_preserves_contract() {
        let diff = non_repo_observed_diff();

        assert!(!diff.data.is_repo);
        assert_eq!(diff.data.summary, SUMMARY_NOT_GIT_REPOSITORY);
        assert_eq!(diff.data.file_count, 0);
        assert!(diff.data.files.is_empty());
        assert_eq!(diff.data.omitted_files, 0);
        assert!(diff.data.patch.is_none());
        assert!(!diff.summary_truncated);
        assert!(!diff.patch_truncated);
    }

    #[test]
    fn transaction_file_summary_reports_truncated_files() {
        assert!(!transaction_files_truncated(0));
        assert_eq!(
            transaction_file_summary("applied patch", "tx-123", 3, 0),
            "applied patch transaction tx-123 touching 3 file(s)"
        );
        assert!(transaction_files_truncated(2));
        assert_eq!(
            transaction_file_summary("rolled back", "tx-123", 3, 2),
            "rolled back transaction tx-123 touching 3 file(s) (files truncated)"
        );
    }

    #[test]
    fn omitted_items_message_reports_counts() {
        assert_eq!(omitted_items_message(0, "file(s)"), None);
        assert_eq!(
            omitted_items_message(2, "dirty file(s)").as_deref(),
            Some("    ... 2 more dirty file(s)")
        );
    }

    #[test]
    fn trailing_newline_helper_preserves_output_rule() {
        assert!(needs_trailing_newline(""));
        assert!(needs_trailing_newline("line"));
        assert!(!needs_trailing_newline("line\n"));
    }

    #[test]
    fn nonblank_trimmed_end_preserves_leading_whitespace() {
        assert_eq!(nonblank_trimmed_end(""), None);
        assert_eq!(nonblank_trimmed_end(" \n\t "), None);
        assert_eq!(nonblank_trimmed_end("  patch\n\n"), Some("  patch"));
    }

    #[test]
    fn append_note_if_preserves_summary_when_condition_is_false() {
        let mut summary = "base".to_string();
        append_note_if(&mut summary, false, " note");
        assert_eq!(summary, "base");
        append_note_if(&mut summary, true, " note");
        assert_eq!(summary, "base note");
    }

    #[test]
    fn run_summary_reports_exit_signal_and_truncation() {
        assert_eq!(
            run_summary(Some(0), 42, false),
            "command exited with 0 in 42ms"
        );
        assert_eq!(
            run_summary(Some(2), 7, true),
            "command exited with 2 in 7ms (output truncated)"
        );
        assert_eq!(
            run_summary(None, 9, false),
            "command exited with signal in 9ms"
        );
    }

    #[test]
    fn read_summary_reports_lines_and_truncation() {
        assert_eq!(read_summary("src/main.rs", None, false), "read src/main.rs");
        assert_eq!(
            read_summary("src/main.rs", Some("3:5"), false),
            "read src/main.rs lines 3:5"
        );
        assert_eq!(
            read_summary("src/main.rs", Some("3:5"), true),
            "read src/main.rs lines 3:5 (truncated)"
        );
    }

    #[test]
    fn log_summary_reports_omitted_lines() {
        let data = LogData {
            log_path: ".workspace/log.jsonl".to_string(),
            omitted_lines: 0,
            entries: vec![],
        };
        assert_eq!(log_summary(&data), "0 operation(s)");
        assert!(!log_lines_omitted(&data));

        let data = LogData {
            log_path: ".workspace/log.jsonl".to_string(),
            omitted_lines: 2,
            entries: vec![LogEntry {
                id: "op-1".to_string(),
                timestamp_unix_ms: 1,
                kind: "observe".to_string(),
                op: "status".to_string(),
                scope: ".".to_string(),
                summary: "entry".to_string(),
                transaction_id: None,
            }],
        };
        assert_eq!(
            log_summary(&data),
            "1 operation(s) (2 older log line(s) omitted)"
        );
        assert!(log_lines_omitted(&data));
    }

    #[test]
    fn execute_run_command_captures_output_without_failing_nonzero_exit() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        #[cfg(windows)]
        let command = "powershell -NoProfile -Command \"[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 7\"";
        #[cfg(not(windows))]
        let command = "printf out; printf err >&2; exit 7";

        let run = execute_run_command(&workspace, command).expect("command should be observed");

        assert_eq!(run.data.command, command);
        assert_eq!(run.data.cwd, temp.path().to_string_lossy());
        assert_eq!(run.data.exit_code, Some(7));
        assert_eq!(run.data.stdout, "out");
        assert_eq!(run.data.stderr, "err");
        assert!(!run.output_truncated);
    }

    #[test]
    fn run_data_helpers_preserve_output_contract() {
        let stdout = CapturedOutput {
            text: "out".to_string(),
            truncated: false,
        };
        let stderr = CapturedOutput {
            text: "err".to_string(),
            truncated: true,
        };

        assert!(captured_outputs_truncated(&stdout, &stderr));

        let data = run_data(
            "printf ok",
            "/repo".to_string(),
            Some(2),
            77,
            stdout.text,
            stderr.text,
        );

        assert_eq!(data.command, "printf ok");
        assert_eq!(data.cwd, "/repo");
        assert_eq!(data.exit_code, Some(2));
        assert_eq!(data.duration_ms, 77);
        assert_eq!(data.stdout, "out");
        assert_eq!(data.stderr, "err");
    }

    #[test]
    fn run_and_log_observation_helpers_preserve_contract() {
        let run_workspace = Workspace {
            root: PathBuf::from("/repo"),
            is_git_repo: false,
        };
        let run = observed_run(
            &run_workspace,
            "printf ok",
            Some(0),
            42,
            CapturedOutput {
                text: "ok".to_string(),
                truncated: false,
            },
            CapturedOutput {
                text: String::new(),
                truncated: true,
            },
        );

        assert_eq!(run.data.cwd, "/repo");
        assert_eq!(run.data.stdout, "ok");
        assert_eq!(run.data.stderr, "");
        assert!(run.output_truncated);

        let observation = run_observation(run);
        assert_eq!(observation.kind, WORKSPACE_RUN_KIND);
        assert_eq!(observation.scope, "printf ok");
        assert_eq!(
            observation.summary,
            "command exited with 0 in 42ms (output truncated)"
        );
        assert!(observation.truncated);
        assert!(observation.evidence.is_empty());
        assert_eq!(observation.next_observations, run_followup_observations());

        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let window = LogWindow {
            entries: vec![LogEntry {
                id: "op-1".to_string(),
                timestamp_unix_ms: 1,
                kind: LOG_KIND_OBSERVE.to_string(),
                op: LOG_OP_STATUS.to_string(),
                scope: ".".to_string(),
                summary: "entry".to_string(),
                transaction_id: None,
            }],
            omitted_lines: 2,
        };

        let data = log_data(&workspace, window);
        assert_eq!(data.log_path, LOG_FILE);
        let observation = log_observation(data);
        assert_eq!(observation.kind, WORKSPACE_LOG_KIND);
        assert_eq!(observation.scope, LOG_FILE);
        assert_eq!(
            observation.summary,
            "1 operation(s) (2 older log line(s) omitted)"
        );
        assert!(observation.truncated);
        assert!(observation.evidence.is_empty());
        assert_eq!(observation.next_observations, log_followup_observations());
    }

    #[test]
    fn observed_log_respects_requested_limit() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        append_operation_log(
            &workspace,
            OperationLogRecord::observe(LOG_OP_MAP, ".", "map"),
        )
        .expect("first log entry should be written");
        append_operation_log(
            &workspace,
            OperationLogRecord::observe(LOG_OP_STATUS, ".", "status"),
        )
        .expect("second log entry should be written");
        append_operation_log(
            &workspace,
            OperationLogRecord::observe(LOG_OP_SEARCH, "needle", "search"),
        )
        .expect("third log entry should be written");
        let args = LogArgs {
            json: true,
            limit: 2,
        };

        let data = observed_log(&workspace, &args).expect("log should be observed");

        assert_eq!(data.log_path, LOG_FILE);
        assert_eq!(data.omitted_lines, 1);
        assert_eq!(data.entries.len(), 2);
        assert_eq!(data.entries[0].op, LOG_OP_STATUS);
        assert_eq!(data.entries[1].op, LOG_OP_SEARCH);
    }

    #[test]
    fn output_best_effort_logged_observation_records_observe_log_before_output() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let observation = Observation {
            kind: WORKSPACE_LOG_KIND.to_string(),
            scope: LOG_FILE.to_string(),
            summary: "test summary".to_string(),
            data: LogData {
                log_path: LOG_FILE.to_string(),
                omitted_lines: 0,
                entries: vec![],
            },
            evidence: vec![],
            truncated: false,
            next_observations: vec![],
        };

        output_best_effort_logged_observation(
            &workspace,
            false,
            LOG_OP_STATUS,
            &observation,
            |_| Ok(()),
        )
        .expect("observation should be logged and output");

        let log = read_log(&workspace, 10).expect("log should be readable");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].kind, LOG_KIND_OBSERVE);
        assert_eq!(log.entries[0].op, LOG_OP_STATUS);
        assert_eq!(log.entries[0].scope, LOG_FILE);
        assert_eq!(log.entries[0].summary, "test summary");
    }

    #[test]
    fn output_required_logged_observation_records_required_observe_log() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let observation = Observation {
            kind: WORKSPACE_INDEX_COCHANGE_KIND.to_string(),
            scope: COCHANGE_INDEX_FILE.to_string(),
            summary: "index summary".to_string(),
            data: IndexCochangeData {
                path: COCHANGE_INDEX_FILE.to_string(),
                version: 1,
                generated_at_unix_ms: 1,
                head: Some("abc123".to_string()),
                max_commits: 10,
                max_files_per_commit: 20,
                commits_scanned: 1,
                commits_indexed: 1,
                ignored_large_commits: 0,
                file_count: 2,
                edge_count: 1,
            },
            evidence: vec![],
            truncated: false,
            next_observations: vec![],
        };

        output_required_logged_observation(
            &workspace,
            false,
            LOG_OP_INDEX_COCHANGE,
            &observation,
            |_| Ok(()),
        )
        .expect("required observation log should be written and output");

        let log = read_log(&workspace, 10).expect("log should be readable");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].kind, LOG_KIND_OBSERVE);
        assert_eq!(log.entries[0].op, LOG_OP_INDEX_COCHANGE);
        assert_eq!(log.entries[0].scope, COCHANGE_INDEX_FILE);
        assert_eq!(log.entries[0].summary, "index summary");
    }

    #[test]
    fn output_recorded_observation_preserves_log_kind_and_transaction() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let observation = Observation {
            kind: WORKSPACE_LOG_KIND.to_string(),
            scope: LOG_FILE.to_string(),
            summary: "observation summary".to_string(),
            data: LogData {
                log_path: LOG_FILE.to_string(),
                omitted_lines: 0,
                entries: vec![],
            },
            evidence: vec![],
            truncated: false,
            next_observations: vec![],
        };

        output_recorded_observation(
            &workspace,
            false,
            OperationLogRecord::change(LOG_OP_PATCH, "change.patch", "custom summary", "tx-1"),
            &observation,
            |_| Ok(()),
        )
        .expect("recorded observation should be logged and output");

        let log = read_log(&workspace, 10).expect("log should be readable");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].kind, LOG_KIND_CHANGE);
        assert_eq!(log.entries[0].op, LOG_OP_PATCH);
        assert_eq!(log.entries[0].scope, "change.patch");
        assert_eq!(log.entries[0].summary, "custom summary");
        assert_eq!(log.entries[0].transaction_id.as_deref(), Some("tx-1"));
    }

    #[test]
    fn semantic_recorded_output_helpers_preserve_log_contracts() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };
        let observation = Observation {
            kind: WORKSPACE_LOG_KIND.to_string(),
            scope: LOG_FILE.to_string(),
            summary: "observation summary".to_string(),
            data: LogData {
                log_path: LOG_FILE.to_string(),
                omitted_lines: 0,
                entries: vec![],
            },
            evidence: vec![],
            truncated: false,
            next_observations: vec![],
        };

        output_changed_observation(
            &workspace,
            false,
            LOG_OP_ROLLBACK,
            "rb-1",
            &observation,
            |_| Ok(()),
        )
        .expect("changed observation should be logged and output");
        output_changed_observation_with_summary(
            &workspace,
            false,
            LOG_OP_PATCH,
            "custom summary",
            "tx-1",
            &observation,
            |_| Ok(()),
        )
        .expect("changed observation with summary should be logged and output");
        output_verified_observation(&workspace, false, LOG_OP_RUN, &observation, |_| Ok(()))
            .expect("verified observation should be logged and output");

        let log = read_log(&workspace, 10).expect("log should be readable");
        assert_eq!(log.entries.len(), 3);
        assert_eq!(log.entries[0].kind, LOG_KIND_CHANGE);
        assert_eq!(log.entries[0].op, LOG_OP_ROLLBACK);
        assert_eq!(log.entries[0].summary, "observation summary");
        assert_eq!(log.entries[0].transaction_id.as_deref(), Some("rb-1"));
        assert_eq!(log.entries[1].kind, LOG_KIND_CHANGE);
        assert_eq!(log.entries[1].op, LOG_OP_PATCH);
        assert_eq!(log.entries[1].summary, "custom summary");
        assert_eq!(log.entries[1].transaction_id.as_deref(), Some("tx-1"));
        assert_eq!(log.entries[2].kind, LOG_KIND_VERIFY);
        assert_eq!(log.entries[2].op, LOG_OP_RUN);
        assert_eq!(log.entries[2].summary, "observation summary");
        assert!(log.entries[2].transaction_id.is_none());
    }

    #[test]
    fn operation_log_record_constructors_preserve_log_contract() {
        let observe = OperationLogRecord::observe(LOG_OP_STATUS, ".", "status summary");
        assert_eq!(observe.kind, LOG_KIND_OBSERVE);
        assert_eq!(observe.op, LOG_OP_STATUS);
        assert_eq!(observe.scope, ".");
        assert_eq!(observe.summary, "status summary");
        assert!(observe.transaction_id.is_none());

        let observation = Observation {
            kind: WORKSPACE_STATUS_KIND.to_string(),
            scope: "status-scope".to_string(),
            summary: "status observation".to_string(),
            data: (),
            evidence: vec![],
            truncated: false,
            next_observations: vec![],
        };
        let observe_from_observation =
            OperationLogRecord::observe_observation(LOG_OP_STATUS, &observation);
        assert_eq!(observe_from_observation.kind, LOG_KIND_OBSERVE);
        assert_eq!(observe_from_observation.op, LOG_OP_STATUS);
        assert_eq!(observe_from_observation.scope, "status-scope");
        assert_eq!(observe_from_observation.summary, "status observation");
        assert!(observe_from_observation.transaction_id.is_none());

        let change = OperationLogRecord::change(LOG_OP_PATCH, "change.patch", "patched", "tx-1");
        assert_eq!(change.kind, LOG_KIND_CHANGE);
        assert_eq!(change.op, LOG_OP_PATCH);
        assert_eq!(change.scope, "change.patch");
        assert_eq!(change.summary, "patched");
        assert_eq!(change.transaction_id, Some("tx-1"));

        let change_from_observation =
            OperationLogRecord::change_observation(LOG_OP_ROLLBACK, &observation, "rb-1");
        assert_eq!(change_from_observation.kind, LOG_KIND_CHANGE);
        assert_eq!(change_from_observation.op, LOG_OP_ROLLBACK);
        assert_eq!(change_from_observation.scope, "status-scope");
        assert_eq!(change_from_observation.summary, "status observation");
        assert_eq!(change_from_observation.transaction_id, Some("rb-1"));

        let change_with_summary = OperationLogRecord::change_observation_summary(
            LOG_OP_PATCH,
            &observation,
            "custom summary",
            "tx-2",
        );
        assert_eq!(change_with_summary.kind, LOG_KIND_CHANGE);
        assert_eq!(change_with_summary.op, LOG_OP_PATCH);
        assert_eq!(change_with_summary.scope, "status-scope");
        assert_eq!(change_with_summary.summary, "custom summary");
        assert_eq!(change_with_summary.transaction_id, Some("tx-2"));

        let verify = OperationLogRecord::verify(LOG_OP_RUN, "cargo test", "command exited with 0");
        assert_eq!(verify.kind, LOG_KIND_VERIFY);
        assert_eq!(verify.op, LOG_OP_RUN);
        assert_eq!(verify.scope, "cargo test");
        assert_eq!(verify.summary, "command exited with 0");
        assert!(verify.transaction_id.is_none());

        let verify_from_observation =
            OperationLogRecord::verify_observation(LOG_OP_RUN, &observation);
        assert_eq!(verify_from_observation.kind, LOG_KIND_VERIFY);
        assert_eq!(verify_from_observation.op, LOG_OP_RUN);
        assert_eq!(verify_from_observation.scope, "status-scope");
        assert_eq!(verify_from_observation.summary, "status observation");
        assert!(verify_from_observation.transaction_id.is_none());
    }

    #[test]
    fn append_operation_log_preserves_record_fields() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        append_operation_log(
            &workspace,
            OperationLogRecord::change(LOG_OP_PATCH, "change.patch", "patched", "tx-1"),
        )
        .expect("operation log should be appended");

        let log = read_log(&workspace, 10).expect("log should be readable");
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].kind, LOG_KIND_CHANGE);
        assert_eq!(log.entries[0].op, LOG_OP_PATCH);
        assert_eq!(log.entries[0].scope, "change.patch");
        assert_eq!(log.entries[0].summary, "patched");
        assert_eq!(log.entries[0].transaction_id.as_deref(), Some("tx-1"));
    }

    #[test]
    fn operation_log_entry_bounds_record_fields() {
        let scope = format!("{}tail", "s".repeat(MAX_LOG_SCOPE + 10));
        let summary = format!("{}tail", "m".repeat(MAX_LOG_SUMMARY + 10));

        let entry = operation_log_entry_with_metadata(
            OperationLogRecord::change(LOG_OP_PATCH, &scope, &summary, "tx-1"),
            "op-test".to_string(),
            42,
        );

        assert_eq!(entry.id, "op-test");
        assert_eq!(entry.timestamp_unix_ms, 42);
        assert_eq!(entry.kind, LOG_KIND_CHANGE);
        assert_eq!(entry.op, LOG_OP_PATCH);
        assert!(entry.scope.contains("[truncated]"));
        assert!(!entry.scope.contains("tail"));
        assert!(entry.scope.chars().count() < MAX_LOG_SCOPE + 20);
        assert!(entry.summary.contains("[truncated]"));
        assert!(!entry.summary.contains("tail"));
        assert!(entry.summary.chars().count() < MAX_LOG_SUMMARY + 20);
        assert_eq!(entry.transaction_id.as_deref(), Some("tx-1"));
    }

    #[test]
    fn store_transaction_patch_for_id_creates_transaction_directory() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: true,
        };
        let source = temp.path().join("change.patch");
        fs::write(&source, "patch content\n").expect("source patch should be written");

        let stored_patch = store_transaction_patch_for_id(&workspace, "tx-42", &source)
            .expect("patch should be stored for transaction");

        assert_eq!(
            stored_patch,
            temp.path().join(TRANSACTION_DIR).join("tx-42.patch")
        );
        assert_eq!(
            fs::read_to_string(&stored_patch).expect("stored patch should be readable"),
            "patch content\n"
        );
    }

    #[test]
    fn transaction_patch_store_replaces_destination_without_leftover_temp() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let source = temp.path().join("change.patch");
        let destination = temp.path().join("tx-1.patch");
        fs::write(&source, "patch content\n").expect("source patch should be written");

        store_transaction_patch(&source, &destination).expect("patch should be stored");
        let leftovers = fs::read_dir(temp.path())
            .expect("temp dir should be readable")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".patch-store-")
            })
            .count();

        assert_eq!(
            fs::read_to_string(&destination).expect("stored patch should be readable"),
            "patch content\n"
        );
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn transaction_patch_temp_create_failure_preserves_existing_temp_file() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let source = temp.path().join("change.patch");
        let temp_path = temp.path().join("tx-1.patch.tmp");
        fs::write(&source, "patch content\n").expect("source patch should be written");
        fs::write(&temp_path, "existing temp").expect("existing temp should be written");

        let Err(error) = copy_file_to_temp_path(&source, &temp_path) else {
            panic!("temp create should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("failed to create stored patch"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read_to_string(&temp_path).expect("existing temp should remain"),
            "existing temp"
        );
    }

    #[test]
    fn fallback_text_search_counts_all_matching_lines() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::write(temp.path().join("b.txt"), "needle three\n").expect("file should be written");
        fs::write(temp.path().join("a.txt"), "needle one\nneedle two\n")
            .expect("file should be written");
        fs::create_dir(temp.path().join(".workspace")).expect("workspace dir should be created");
        fs::write(temp.path().join(".workspace/log.jsonl"), "needle ignored\n")
            .expect("log should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let (matches, total_matches, truncated_match_texts) =
            fallback_text_search(&workspace, "needle", 2).expect("fallback search should work");

        assert_eq!(total_matches, 3);
        assert_eq!(truncated_match_texts, 0);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "a.txt");
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[0].column, 1);
    }

    #[test]
    fn fallback_text_search_truncates_large_matching_lines() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let line = format!("needle {} tail\n", "a".repeat(30_000));
        fs::write(temp.path().join("large.txt"), &line).expect("file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let (matches, total_matches, truncated_match_texts) =
            fallback_text_search(&workspace, "needle", 10).expect("fallback search should work");

        assert_eq!(total_matches, 1);
        assert_eq!(truncated_match_texts, 1);
        assert_eq!(matches.len(), 1);
        assert!(matches[0].text.contains("[output truncated]"));
        assert!(!matches[0].text.contains("tail"));
    }

    #[test]
    fn fallback_text_search_matches_across_read_buffer() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let line = format!("{}needle\n", "a".repeat(8190));
        fs::write(temp.path().join("large.txt"), &line).expect("file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let (matches, total_matches, truncated_match_texts) =
            fallback_text_search(&workspace, "needle", 10).expect("fallback search should work");

        assert_eq!(total_matches, 1);
        assert_eq!(truncated_match_texts, 1);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[0].column, 8191);
    }

    #[test]
    fn fallback_query_scan_matches_across_segments() {
        let mut line = FallbackLineSearch::with_display(1, true);
        fallback_scan_query(&mut line, b"abc", b"cde");
        line.byte_offset += 3;

        assert!(!line.matched);
        assert_eq!(line.scan_tail, b"bc");

        fallback_scan_query(&mut line, b"def", b"cde");

        assert!(line.matched);
        assert_eq!(line.match_column, 3);
        assert_eq!(line.scan_tail, b"ef");
    }

    #[test]
    fn fallback_text_search_skips_invalid_utf8_files() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::write(temp.path().join("invalid.bin"), b"needle \xff\n")
            .expect("file should be written");
        fs::write(temp.path().join("valid.txt"), "needle valid\n").expect("file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let (matches, total_matches, truncated_match_texts) =
            fallback_text_search(&workspace, "needle", 10).expect("fallback search should work");

        assert_eq!(total_matches, 1);
        assert_eq!(truncated_match_texts, 0);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "valid.txt");
    }

    #[test]
    fn fallback_text_search_count_only_still_skips_invalid_utf8_files() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        fs::write(temp.path().join("invalid.bin"), b"needle \xff\n")
            .expect("file should be written");
        fs::write(temp.path().join("valid.txt"), "needle valid\n").expect("file should be written");
        let workspace = Workspace {
            root: temp.path().to_path_buf(),
            is_git_repo: false,
        };

        let (matches, total_matches, truncated_match_texts) =
            fallback_text_search(&workspace, "needle", 0).expect("fallback search should work");

        assert_eq!(total_matches, 1);
        assert_eq!(truncated_match_texts, 0);
        assert!(matches.is_empty());
    }

    #[test]
    fn ripgrep_json_parser_rejects_oversized_lines_after_draining() {
        let json = format!(
            "{}\n{{\"type\":\"match\",\"data\":{{\"path\":{{\"text\":\"./a.txt\"}},\"lines\":{{\"text\":\"needle\\n\"}},\"line_number\":1,\"submatches\":[{{\"start\":0}}]}}}}\n",
            "x".repeat(MAX_RG_JSON_LINE_BYTES + 1)
        );

        let Err(error) = parse_rg_json_output(std::io::Cursor::new(json), 10) else {
            panic!("oversized ripgrep JSON line should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("ripgrep JSON line 1"),
            "unexpected error: {error}"
        );
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn reads_only_requested_log_window() {
        let log = "\
not json
{\"id\":\"op-1\",\"timestamp_unix_ms\":1,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"one\",\"transaction_id\":null}
{\"id\":\"op-2\",\"timestamp_unix_ms\":2,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"two\",\"transaction_id\":null}
";
        let window =
            read_log_entries(std::io::Cursor::new(log), 2).expect("log window should parse");

        assert_eq!(window.entries.len(), 2);
        assert_eq!(window.omitted_lines, 1);
        assert_eq!(window.entries[0].id, "op-1");
        assert_eq!(window.entries[1].id, "op-2");
    }

    #[test]
    fn ignores_oversized_log_lines_outside_requested_window() {
        let log = format!(
            "{}\n{{\"id\":\"op-1\",\"timestamp_unix_ms\":1,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"one\",\"transaction_id\":null}}\n{{\"id\":\"op-2\",\"timestamp_unix_ms\":2,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"two\",\"transaction_id\":null}}\n",
            "x".repeat(MAX_LOG_LINE_BYTES + 1)
        );

        let window =
            read_log_entries(std::io::Cursor::new(log), 2).expect("log window should parse");

        assert_eq!(window.entries.len(), 2);
        assert_eq!(window.omitted_lines, 1);
        assert_eq!(window.entries[0].id, "op-1");
        assert_eq!(window.entries[1].id, "op-2");
    }

    #[test]
    fn rejects_oversized_log_lines_inside_requested_window() {
        let log = format!("{}\n", "x".repeat(MAX_LOG_LINE_BYTES + 1));

        let Err(error) = read_log_entries(std::io::Cursor::new(log), 1) else {
            panic!("oversized log line should fail");
        };
        let error = format!("{error:#}");

        assert!(error.contains("line 1"), "unexpected error: {error}");
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn zero_log_limit_skips_parsing() {
        let window = read_log_entries(std::io::Cursor::new("not json\n"), 0)
            .expect("zero limit should parse");

        assert!(window.entries.is_empty());
        assert_eq!(window.omitted_lines, 0);
    }

    #[test]
    fn excludes_non_observable_repo_paths() {
        assert!(should_include_repo_file("src/main.rs"));
        assert!(should_include_repo_file("space name.txt"));
        assert!(should_include_repo_file("src/has:colon.rs"));
        assert!(!should_include_repo_file(".workspace/log.jsonl"));
        assert!(!should_include_repo_file(".git/config"));
        assert!(!should_include_repo_file("../outside.rs"));
        assert!(!should_include_repo_file("src/../outside.rs"));
        assert!(!should_include_repo_file("/tmp/outside.rs"));
        assert!(!should_include_repo_file("C:/outside.rs"));
        assert!(!should_include_repo_file("z:/outside.rs"));
        assert!(!should_include_repo_file("src//main.rs"));
    }

    #[test]
    fn detects_structure_in_stable_priority_order() {
        let mut signals = MapSignals::default();
        let files = vec![
            "tests/z_test.rs".to_string(),
            "index.js".to_string(),
            "README.md".to_string(),
            "vite.config.js".to_string(),
            "src/main.rs".to_string(),
            "tests/a_test.rs".to_string(),
            "Cargo.toml".to_string(),
            "docs/guide.md".to_string(),
        ];
        for file in files {
            record_map_file(&mut signals, &file);
        }
        signals.directories.push("z".to_string());
        signals.directories.push("a".to_string());
        let (structure, omitted) = detect_structure(&signals);

        assert_eq!(structure.directories, vec!["a", "z"]);
        assert_eq!(structure.entrypoints, vec!["src/main.rs", "index.js"]);
        assert_eq!(structure.configs, vec!["Cargo.toml", "vite.config.js"]);
        assert_eq!(structure.tests, vec!["tests/a_test.rs", "tests/z_test.rs"]);
        assert_eq!(structure.docs, vec!["README.md", "docs/guide.md"]);
        assert!(!omitted.any());
    }

    #[test]
    fn finds_package_json_frameworks_across_read_buffer() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let package_json = temp.path().join("package.json");
        let content = format!("{}\"react\": \"latest\"\n", "x".repeat(8189));
        fs::write(&package_json, content).expect("package.json should be written");

        let frameworks =
            detect_package_json_frameworks(&package_json).expect("frameworks should be detected");

        assert_eq!(frameworks, vec!["react"]);
    }

    #[test]
    fn rejects_oversized_json_before_parsing() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let path = temp.path().join("package.json");
        let file = fs::File::create(&path).expect("package.json should be created");
        file.set_len(MAX_PACKAGE_JSON_BYTES + 1)
            .expect("package.json size should be set");

        let Err(error) = read_json_file_bounded(&path, MAX_PACKAGE_JSON_BYTES) else {
            panic!("oversized JSON should fail before parsing");
        };
        let error = format!("{error:#}");

        assert!(error.contains("JSON file"), "unexpected error: {error}");
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn detect_commands_skips_oversized_package_json() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let root = temp.path();
        let package_json = root.join("package.json");
        let file = fs::File::create(&package_json).expect("package.json should be created");
        file.set_len(MAX_PACKAGE_JSON_BYTES + 1)
            .expect("package.json size should be set");
        let workspace = Workspace {
            root: root.to_path_buf(),
            is_git_repo: false,
        };
        let mut signals = MapSignals::default();
        signals.named_files.insert("package.json".to_string());

        let commands = detect_commands(&workspace, &signals).expect("commands should be detected");

        assert!(commands.is_empty());
    }

    #[test]
    fn keeps_recent_file_candidates_bounded() {
        let mut candidates = Vec::new();
        for index in 0..20 {
            push_recent_candidate(
                &mut candidates,
                UNIX_EPOCH + std::time::Duration::from_secs(index),
                format!("file_{index:03}.txt"),
            );
        }

        assert_eq!(candidates.len(), MAX_RECENT_FILES);
        assert_eq!(candidates[0].1, "file_019.txt");
        assert_eq!(candidates[MAX_RECENT_FILES - 1].1, "file_008.txt");
    }

    #[test]
    fn orders_recent_file_candidates_deterministically_on_time_ties() {
        let mut candidates = Vec::new();
        push_recent_candidate(&mut candidates, UNIX_EPOCH, "b.txt".to_string());
        push_recent_candidate(&mut candidates, UNIX_EPOCH, "a.txt".to_string());

        assert_eq!(candidates[0].1, "a.txt");
        assert_eq!(candidates[1].1, "b.txt");
    }

    #[test]
    fn keeps_map_item_candidates_bounded_and_sorted() {
        let mut items = BoundedMapItems::default();
        for index in (0..90).rev() {
            items.push(format!("item_{index:03}"));
        }

        let observed = items.observed();
        assert_eq!(items.total_items(), 90);
        assert_eq!(observed.len(), MAX_MAP_LIST_ITEMS);
        assert_eq!(items.omitted_count(), 10);
        assert_eq!(observed[0], "item_000");
        assert_eq!(observed[MAX_MAP_LIST_ITEMS - 1], "item_079");
    }

    #[test]
    fn keeps_large_file_candidates_bounded_and_sorted() {
        let mut large_files = Vec::new();
        for index in 0..45 {
            push_large_file_candidate(
                &mut large_files,
                LargeFile {
                    path: format!("file_{index:03}.bin"),
                    bytes: 1_000_000 + index,
                },
            );
        }

        assert_eq!(large_files.len(), MAX_MAP_LARGE_FILES);
        assert_eq!(large_files[0].path, "file_044.bin");
        assert_eq!(large_files[MAX_MAP_LARGE_FILES - 1].path, "file_005.bin");
    }

    #[test]
    fn pushes_bounded_sorted_items_by_insertion_position() {
        let mut items = Vec::new();
        for item in [3, 1, 4, 2, 0] {
            push_bounded_sorted(&mut items, item, 3, |a, b| a.cmp(b));
        }

        assert_eq!(items, vec![0, 1, 2]);

        push_bounded_sorted(&mut items, 9, 3, |a, b| a.cmp(b));
        assert_eq!(items, vec![0, 1, 2]);
    }

    #[test]
    fn sort_and_truncate_orders_and_bounds_items() {
        let mut items = vec![3, 1, 4, 2, 0];
        sort_and_truncate(&mut items, 3, |a, b| a.cmp(b));

        assert_eq!(items, vec![0, 1, 2]);

        sort_and_truncate(&mut items, 0, |a, b| a.cmp(b));
        assert!(items.is_empty());
    }

    #[test]
    fn parses_git_log_name_only() {
        let log = "\
commit:aaaaaaaaaaaa

src/a.rs
src/b.rs

commit:bbbbbbbbbbbb
src/a.rs
.workspace/log.jsonl
src/c.rs
";
        let commits = parse_git_log_name_only(log);

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "aaaaaaaaaaaa");
        assert_eq!(commits[0].files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!(commits[1].files, vec!["src/a.rs", "src/c.rs"]);
    }

    #[test]
    fn reads_git_log_name_only_incrementally() {
        let log = "\
commit:aaaaaaaaaaaa
\"src/tab\\tname.rs\"
src/a.rs

commit:bbbbbbbbbbbb
.git/config
src/b.rs
";
        let commits =
            read_git_log_name_only(std::io::Cursor::new(log)).expect("git log should parse");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "aaaaaaaaaaaa");
        assert_eq!(commits[0].files, vec!["src/a.rs", "src/tab\tname.rs"]);
        assert_eq!(commits[1].files, vec!["src/b.rs"]);
    }

    #[test]
    fn rejects_oversized_git_log_lines() {
        let log = format!(
            "commit:aaaaaaaaaaaa\n{}\n",
            "x".repeat(MAX_GIT_OUTPUT_LINE_BYTES + 1)
        );

        let Err(error) = read_git_log_name_only(std::io::Cursor::new(log)) else {
            panic!("oversized git log line should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("git log output line 2"),
            "unexpected error: {error}"
        );
        assert!(error.contains("exceeded"), "unexpected error: {error}");
    }

    #[test]
    fn ranks_cochanged_files() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec![
                    "src/a.rs".to_string(),
                    "src/b.rs".to_string(),
                    "tests/a_test.rs".to_string(),
                ],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "cccccccccccc".to_string(),
                files: vec!["src/other.rs".to_string(), "src/b.rs".to_string()],
            },
        ];

        let ranking = rank_cochanges(&commits, "src/a.rs", 10, 10);

        assert_eq!(ranking.commits_matched, 2);
        assert_eq!(ranking.ignored_large_commits, 0);
        assert_eq!(ranking.related[0].path, "src/b.rs");
        assert_eq!(ranking.related[0].cochanged_commits, 2);
        assert_eq!(ranking.related[1].path, "tests/a_test.rs");
    }

    #[test]
    fn bounds_related_cli_files_after_filtering_invalid_paths() {
        let related = bounded_related_cli_files(
            vec![
                RelatedCliItem {
                    path: ".workspace/log.jsonl".to_string(),
                    score: 1.0,
                    cochanges: 9,
                    weight: 9.0,
                    evidence: vec![],
                },
                RelatedCliItem {
                    path: "../outside.rs".to_string(),
                    score: 0.9,
                    cochanges: 8,
                    weight: 8.0,
                    evidence: vec![],
                },
                RelatedCliItem {
                    path: "src/b.rs".to_string(),
                    score: 0.8,
                    cochanges: 2,
                    weight: 1.5,
                    evidence: vec![crate::related_cli::RelatedCliEvidence {
                        hash: "aaaaaaaaaaaa".to_string(),
                    }],
                },
                RelatedCliItem {
                    path: "src/c.rs".to_string(),
                    score: 0.7,
                    cochanges: 1,
                    weight: 1.0,
                    evidence: vec![],
                },
            ],
            1,
        );

        assert_eq!(related.len(), 1);
        assert_eq!(related[0].path, "src/b.rs");
        assert_eq!(related[0].sample_commits, vec!["aaaaaaaaaaaa"]);
    }

    #[test]
    fn ranks_cochanged_files_with_bounded_results() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "cccccccccccc".to_string(),
                files: vec!["src/a.rs".to_string(), "src/c.rs".to_string()],
            },
        ];

        let ranking = rank_cochanges(&commits, "src/a.rs", 10, 1);

        assert_eq!(ranking.related.len(), 1);
        assert_eq!(ranking.related[0].path, "src/b.rs");
        assert_eq!(ranking.related[0].cochanged_commits, 2);
    }

    #[test]
    fn ignores_large_cochange_commits() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string(),
            ],
        }];

        let ranking = rank_cochanges(&commits, "src/a.rs", 2, 10);

        assert_eq!(ranking.commits_matched, 1);
        assert_eq!(ranking.ignored_large_commits, 1);
        assert!(ranking.related.is_empty());
    }

    #[test]
    fn cochange_ranking_counts_unique_files_for_broad_commits() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/b.rs".to_string(),
            ],
        }];

        let ranking = rank_cochanges(&commits, "src/a.rs", 2, 10);

        assert_eq!(ranking.commits_matched, 1);
        assert_eq!(ranking.ignored_large_commits, 0);
        assert_eq!(ranking.related.len(), 1);
        assert_eq!(ranking.related[0].path, "src/b.rs");
    }

    #[test]
    fn ignores_unmatched_large_cochange_commits_without_counting_them() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/other.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string(),
            ],
        }];

        let ranking = rank_cochanges(&commits, "src/a.rs", 2, 10);

        assert_eq!(ranking.commits_matched, 0);
        assert_eq!(ranking.ignored_large_commits, 0);
        assert!(ranking.related.is_empty());
    }

    #[test]
    fn ranks_impact_from_multiple_seed_files() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "tests/a_test.rs".to_string()],
            },
            GitCommitFiles {
                hash: "cccccccccccc".to_string(),
                files: vec!["src/other.rs".to_string(), "src/b.rs".to_string()],
            },
        ];
        let seeds = vec!["src/a.rs".to_string(), "src/other.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 10, 10);

        assert_eq!(ranking.commits_matched, 3);
        assert_eq!(ranking.impacted[0].path, "src/b.rs");
        assert_eq!(ranking.impacted[0].cochanged_commits, 2);
        assert_eq!(
            ranking.impacted[0].seed_files,
            vec!["src/a.rs", "src/other.rs"]
        );
        assert_eq!(ranking.impacted[1].path, "tests/a_test.rs");
    }

    #[test]
    fn ranks_impact_with_bounded_results() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "cccccccccccc".to_string(),
                files: vec!["src/a.rs".to_string(), "src/c.rs".to_string()],
            },
        ];
        let seeds = vec!["src/a.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 10, 1);

        assert_eq!(ranking.impacted.len(), 1);
        assert_eq!(ranking.impacted[0].path, "src/b.rs");
        assert_eq!(ranking.impacted[0].cochanged_commits, 2);
    }

    #[test]
    fn ignores_large_impact_commits_after_matching_seeds() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string(),
            ],
        }];
        let seeds = vec!["src/a.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 2, 10);

        assert_eq!(ranking.commits_matched, 1);
        assert_eq!(ranking.ignored_large_commits, 1);
        assert!(ranking.impacted.is_empty());
    }

    #[test]
    fn impact_ranking_counts_unique_files_for_broad_commits() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/b.rs".to_string(),
            ],
        }];
        let seeds = vec!["src/a.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 2, 10);

        assert_eq!(ranking.commits_matched, 1);
        assert_eq!(ranking.ignored_large_commits, 0);
        assert_eq!(ranking.impacted.len(), 1);
        assert_eq!(ranking.impacted[0].path, "src/b.rs");
    }

    #[test]
    fn ignores_unmatched_large_impact_commits_without_counting_them() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/other.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string(),
            ],
        }];
        let seeds = vec!["src/a.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 2, 10);

        assert_eq!(ranking.commits_matched, 0);
        assert_eq!(ranking.ignored_large_commits, 0);
        assert!(ranking.impacted.is_empty());
    }

    #[test]
    fn impact_excludes_seed_files() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        }];
        let seeds = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let ranking = rank_cochange_impact(&commits, &seeds, 10, 10);

        assert!(ranking.impacted.is_empty());
    }

    #[test]
    fn builds_cochange_index_from_commits() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "tests/a_test.rs".to_string()],
            },
        ];

        let index = cochange_index_from_commits(&commits, 100, 10, Some("head".to_string()));

        assert_eq!(index.version, 1);
        assert_eq!(index.commits_scanned, 2);
        assert_eq!(index.commits_indexed, 2);
        assert_eq!(index.file_commit_counts["src/a.rs"], 2);
        assert_eq!(index.edges.len(), 2);
    }

    #[test]
    fn cochange_index_ignores_broad_commits_without_indexing_counts() {
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string(),
            ],
        }];

        let index = cochange_index_from_commits(&commits, 100, 2, Some("head".to_string()));

        assert_eq!(index.commits_scanned, 1);
        assert_eq!(index.commits_indexed, 0);
        assert_eq!(index.ignored_large_commits, 1);
        assert!(index.file_commit_counts.is_empty());
        assert!(index.edges.is_empty());
    }

    #[test]
    fn cochange_index_round_trips_through_json_file() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let path = temp.path().join("cochange.json");
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        }];
        let index = cochange_index_from_commits(&commits, 100, 10, Some("head".to_string()));

        write_cochange_index(&path, &index).expect("index should be written");
        let loaded = read_cochange_index_from_path(&path).expect("index should be read");

        assert_eq!(loaded.version, index.version);
        assert_eq!(loaded.head, index.head);
        assert_eq!(loaded.file_commit_counts, index.file_commit_counts);
        assert_eq!(loaded.edges.len(), index.edges.len());
    }

    #[test]
    fn cochange_index_write_replaces_existing_file_without_leftover_temp() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let path = temp.path().join("cochange.json");
        let old_commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec!["src/old.rs".to_string(), "src/shared.rs".to_string()],
        }];
        let new_commits = vec![GitCommitFiles {
            hash: "bbbbbbbbbbbb".to_string(),
            files: vec!["src/new.rs".to_string(), "src/shared.rs".to_string()],
        }];
        let old_index = cochange_index_from_commits(&old_commits, 100, 10, Some("old".to_string()));
        let new_index = cochange_index_from_commits(&new_commits, 100, 10, Some("new".to_string()));

        write_cochange_index(&path, &old_index).expect("old index should be written");
        write_cochange_index(&path, &new_index).expect("new index should replace old index");
        let loaded = read_cochange_index_from_path(&path).expect("index should be read");
        let leftovers = fs::read_dir(temp.path())
            .expect("temp dir should be readable")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".cochange-index-")
            })
            .count();

        assert_eq!(loaded.head, Some("new".to_string()));
        assert_eq!(loaded.file_commit_counts, new_index.file_commit_counts);
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn cochange_index_temp_create_failure_preserves_existing_temp_file() {
        let temp = tempfile::TempDir::new().expect("temp dir should be created");
        let temp_path = temp.path().join("cochange.json.tmp");
        fs::write(&temp_path, "existing temp").expect("existing temp should be written");
        let commits = vec![GitCommitFiles {
            hash: "aaaaaaaaaaaa".to_string(),
            files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        }];
        let index = cochange_index_from_commits(&commits, 100, 10, Some("head".to_string()));

        let Err(error) = write_cochange_index_temp(&temp_path, &index) else {
            panic!("temp create should fail");
        };
        let error = format!("{error:#}");

        assert!(
            error.contains("failed to create temporary co-change index"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read_to_string(&temp_path).expect("existing temp should remain"),
            "existing temp"
        );
    }

    #[test]
    fn ranks_related_from_cochange_index() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "cccccccccccc".to_string(),
                files: vec!["src/a.rs".to_string(), "tests/a_test.rs".to_string()],
            },
        ];
        let index = cochange_index_from_commits(&commits, 100, 10, None);

        let ranking = rank_cochanges_from_index(&index, "src/a.rs", 10);

        assert_eq!(ranking.commits_matched, 3);
        assert_eq!(ranking.related[0].path, "src/b.rs");
        assert_eq!(ranking.related[0].cochanged_commits, 2);
    }

    #[test]
    fn ranks_impact_from_cochange_index() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/other.rs".to_string(), "src/b.rs".to_string()],
            },
        ];
        let index = cochange_index_from_commits(&commits, 100, 10, None);
        let seeds = vec!["src/a.rs".to_string(), "src/other.rs".to_string()];

        let ranking = rank_cochange_impact_from_index(&index, &seeds, 10);

        assert_eq!(ranking.impacted[0].path, "src/b.rs");
        assert_eq!(
            ranking.impacted[0].seed_files,
            vec!["src/a.rs", "src/other.rs"]
        );
    }

    #[test]
    fn cochange_edge_lookup_preserves_first_matching_edge() {
        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 0,
            head: None,
            max_commits: 10,
            max_files_per_commit: 10,
            commits_scanned: 2,
            commits_indexed: 2,
            ignored_large_commits: 0,
            file_commit_counts: BTreeMap::new(),
            edges: vec![
                CochangeEdge {
                    a: "src/a.rs".to_string(),
                    b: "src/b.rs".to_string(),
                    cochanged_commits: 1,
                    weighted_cochanges: 1.0,
                    sample_commits: vec!["aaaaaaaaaaaa".to_string()],
                },
                CochangeEdge {
                    a: "src/b.rs".to_string(),
                    b: "src/a.rs".to_string(),
                    cochanged_commits: 2,
                    weighted_cochanges: 2.0,
                    sample_commits: vec!["bbbbbbbbbbbb".to_string()],
                },
            ],
        };

        let lookup = cochange_edge_lookup(&index);
        let edge =
            find_cochange_edge(&lookup, "src/b.rs", "src/a.rs").expect("edge should be found");

        assert_eq!(edge.cochanged_commits, 1);
        assert_eq!(edge.sample_commits, vec!["aaaaaaaaaaaa"]);
    }

    #[test]
    fn pagerank_reaches_indirect_related_files() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/b.rs".to_string(), "src/c.rs".to_string()],
            },
        ];
        let index = cochange_index_from_commits(&commits, 100, 10, None);

        let ranking = rank_cochanges_pagerank_from_index(&index, "src/a.rs", 10);

        assert_eq!(ranking.related[0].path, "src/b.rs");
        assert!(ranking.related.iter().any(|file| file.path == "src/c.rs"));
        let indirect = ranking
            .related
            .iter()
            .find(|file| file.path == "src/c.rs")
            .unwrap();
        assert_eq!(indirect.cochanged_commits, 0);
        assert!(indirect.score > 0.0);
    }

    #[test]
    fn pagerank_impact_reaches_indirect_files() {
        let commits = vec![
            GitCommitFiles {
                hash: "aaaaaaaaaaaa".to_string(),
                files: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            },
            GitCommitFiles {
                hash: "bbbbbbbbbbbb".to_string(),
                files: vec!["src/b.rs".to_string(), "src/c.rs".to_string()],
            },
        ];
        let index = cochange_index_from_commits(&commits, 100, 10, None);
        let seeds = vec!["src/a.rs".to_string()];

        let ranking = rank_cochange_impact_pagerank_from_index(&index, &seeds, 10);

        assert_eq!(ranking.impacted[0].path, "src/b.rs");
        assert!(ranking.impacted.iter().any(|file| file.path == "src/c.rs"));
        let indirect = ranking
            .impacted
            .iter()
            .find(|file| file.path == "src/c.rs")
            .unwrap();
        assert_eq!(indirect.cochanged_commits, 0);
        assert_eq!(indirect.seed_files, vec!["src/a.rs"]);
    }

    #[test]
    fn sample_commit_helpers_bound_and_deduplicate() {
        let mut samples = Vec::new();
        for index in 0..(MAX_SAMPLE_COMMITS + 1) {
            let pushed = push_sample_commit(&mut samples, format!("commit-{index}"));
            assert_eq!(pushed, index < MAX_SAMPLE_COMMITS);
        }
        assert_eq!(samples.len(), MAX_SAMPLE_COMMITS);

        let mut unique_samples = vec!["commit-a".to_string()];
        assert!(push_unique_sample_commit(&mut unique_samples, "commit-a"));
        assert_eq!(unique_samples, vec!["commit-a"]);
        assert!(push_unique_sample_commit(&mut unique_samples, "commit-b"));
        assert_eq!(unique_samples, vec!["commit-a", "commit-b"]);
    }

    #[test]
    fn related_cli_sample_commit_helpers_bound_evidence() {
        let evidence = (0..(MAX_SAMPLE_COMMITS + 1))
            .map(|index| RelatedCliEvidence {
                hash: format!("commit-{index}-abcdef"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            related_cli_sample_commits(&evidence),
            vec![
                "commit-0-abc",
                "commit-1-abc",
                "commit-2-abc",
                "commit-3-abc",
                "commit-4-abc"
            ]
        );

        let mut samples = vec!["existing".to_string()];
        push_related_cli_sample_commits(&mut samples, &evidence);
        assert_eq!(
            samples,
            vec![
                "existing",
                "commit-0-abc",
                "commit-1-abc",
                "commit-2-abc",
                "commit-3-abc"
            ]
        );
    }

    #[test]
    fn ranking_weight_helpers_preserve_expected_scoring() {
        assert_eq!(round3(cochange_commit_weight(0, 2)), 0.91);
        assert_eq!(round3(cochange_commit_weight(50, 4)), 0.311);
        assert_eq!(round3(cochange_commit_weight(0, 1)), 0.91);

        assert_eq!(impact_seed_weight(0), 1.0);
        assert_eq!(impact_seed_weight(1), 1.0);
        assert_eq!(impact_seed_weight(3), 1.5);
    }

    #[test]
    fn normalized_rank_score_rounds_and_handles_zero_max() {
        assert_eq!(normalized_rank_score(2.0, 4.0), 0.5);
        assert_eq!(normalized_rank_score(1.0, 3.0), 0.333);
        assert_eq!(normalized_rank_score(10.0, 0.0), 0.0);
    }

    #[test]
    fn max_rank_weight_returns_zero_for_empty_inputs() {
        assert_eq!(max_rank_weight([]), 0.0);
        assert_eq!(max_rank_weight([0.25, 1.5, 0.75]), 1.5);
    }

    #[test]
    fn seed_file_helpers_normalize_and_count_indexed_commits() {
        let seeds = vec![
            "./src/a.rs".to_string(),
            "src/a.rs".to_string(),
            "src\\b.rs".to_string(),
            "  ".to_string(),
        ];
        let seed_set = normalized_seed_file_set(&seeds);

        assert_eq!(
            seed_set.iter().cloned().collect::<Vec<_>>(),
            vec!["src/a.rs", "src/b.rs"]
        );

        let mut file_commit_counts = BTreeMap::new();
        file_commit_counts.insert("src/a.rs".to_string(), 2);
        file_commit_counts.insert("src/b.rs".to_string(), 3);
        file_commit_counts.insert("src/c.rs".to_string(), 5);
        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 0,
            head: None,
            max_commits: 10,
            max_files_per_commit: 10,
            commits_scanned: 0,
            commits_indexed: 0,
            ignored_large_commits: 0,
            file_commit_counts,
            edges: vec![],
        };

        assert_eq!(indexed_file_commit_count(&index, "src/a.rs"), 2);
        assert_eq!(indexed_file_commit_count(&index, "src/missing.rs"), 0);
        assert_eq!(indexed_seed_commit_count(&index, &seed_set), 5);
    }

    #[test]
    fn impact_file_from_accumulator_preserves_rank_fields() {
        let mut seed_files = BTreeSet::new();
        seed_files.insert("src/z.rs".to_string());
        seed_files.insert("src/a.rs".to_string());
        let file = impact_file_from_accumulator(
            "src/impact.rs".to_string(),
            ImpactAccumulator {
                cochanged_commits: 3,
                weighted_cochanges: 2.0 / 3.0,
                seed_files,
                sample_commits: vec!["aaaaaaaaaaaa".to_string()],
            },
            2.0,
        );

        assert_eq!(file.path, "src/impact.rs");
        assert_eq!(file.score, 0.333);
        assert_eq!(file.cochanged_commits, 3);
        assert_eq!(file.weighted_cochanges, 0.667);
        assert_eq!(file.seed_files, vec!["src/a.rs", "src/z.rs"]);
        assert_eq!(file.sample_commits, vec!["aaaaaaaaaaaa"]);
    }

    #[test]
    fn impact_file_from_related_cli_accumulator_preserves_rank_fields() {
        let mut seed_files = BTreeSet::new();
        seed_files.insert("src/z.rs".to_string());
        seed_files.insert("src/a.rs".to_string());
        let file = impact_file_from_related_cli_accumulator(
            "src/impact.rs".to_string(),
            RelatedCliImpactAccumulator {
                score: 2.0 / 3.0,
                cochanged_commits: 4,
                weighted_cochanges: 5.0 / 3.0,
                seed_files,
                sample_commits: vec!["aaaaaaaaaaaa".to_string()],
            },
            2.0,
        );

        assert_eq!(file.path, "src/impact.rs");
        assert_eq!(file.score, 0.333);
        assert_eq!(file.cochanged_commits, 4);
        assert_eq!(file.weighted_cochanges, 1.667);
        assert_eq!(file.seed_files, vec!["src/a.rs", "src/z.rs"]);
        assert_eq!(file.sample_commits, vec!["aaaaaaaaaaaa"]);
    }

    #[test]
    fn impact_pagerank_hit_conversion_preserves_direct_edges() {
        let index = CochangeIndex {
            version: 1,
            generated_at_unix_ms: 0,
            head: None,
            max_commits: 10,
            max_files_per_commit: 10,
            commits_scanned: 0,
            commits_indexed: 0,
            ignored_large_commits: 0,
            file_commit_counts: BTreeMap::new(),
            edges: vec![
                CochangeEdge {
                    a: "src/a.rs".to_string(),
                    b: "src/impact.rs".to_string(),
                    cochanged_commits: 2,
                    weighted_cochanges: 1.25,
                    sample_commits: vec!["aaaaaaaaaaaa".to_string()],
                },
                CochangeEdge {
                    a: "src/other.rs".to_string(),
                    b: "src/impact.rs".to_string(),
                    cochanged_commits: 3,
                    weighted_cochanges: 0.75,
                    sample_commits: vec!["aaaaaaaaaaaa".to_string(), "bbbbbbbbbbbb".to_string()],
                },
            ],
        };
        let edge_lookup = cochange_edge_lookup(&index);
        let seed_files = BTreeSet::from(["src/a.rs".to_string(), "src/other.rs".to_string()]);

        let file = impact_file_from_pagerank_hit(
            PageRankHit {
                path: "src/impact.rs".to_string(),
                score: 2.0 / 3.0,
            },
            &seed_files,
            &edge_lookup,
        );

        assert_eq!(file.path, "src/impact.rs");
        assert_eq!(file.score, 0.667);
        assert_eq!(file.cochanged_commits, 5);
        assert_eq!(file.weighted_cochanges, 2.0);
        assert_eq!(file.seed_files, vec!["src/a.rs", "src/other.rs"]);
        assert_eq!(file.sample_commits, vec!["aaaaaaaaaaaa", "bbbbbbbbbbbb"]);

        let file = impact_file_from_pagerank_hit(
            PageRankHit {
                path: "src/indirect.rs".to_string(),
                score: 0.25,
            },
            &seed_files,
            &edge_lookup,
        );

        assert_eq!(file.path, "src/indirect.rs");
        assert_eq!(file.score, 0.25);
        assert_eq!(file.cochanged_commits, 0);
        assert_eq!(file.weighted_cochanges, 0.0);
        assert_eq!(file.seed_files, vec!["src/a.rs", "src/other.rs"]);
        assert!(file.sample_commits.is_empty());
    }

    #[test]
    fn related_file_helpers_preserve_rank_fields() {
        let file = related_file_from_accumulator(
            "src/related.rs".to_string(),
            CochangeAccumulator {
                cochanged_commits: 2,
                weighted_cochanges: 2.0 / 3.0,
                sample_commits: vec!["aaaaaaaaaaaa".to_string()],
            },
            2.0,
        );

        assert_eq!(file.path, "src/related.rs");
        assert_eq!(file.score, 0.333);
        assert_eq!(file.cochanged_commits, 2);
        assert_eq!(file.weighted_cochanges, 0.667);
        assert_eq!(file.sample_commits, vec!["aaaaaaaaaaaa"]);

        let edge = CochangeEdge {
            a: "src/a.rs".to_string(),
            b: "src/b.rs".to_string(),
            cochanged_commits: 4,
            weighted_cochanges: 1.25,
            sample_commits: vec!["bbbbbbbbbbbb".to_string()],
        };
        let file = related_file_from_edge("src/b.rs".to_string(), &edge, 2.5);

        assert_eq!(file.score, 0.5);
        assert_eq!(file.cochanged_commits, 4);
        assert_eq!(file.weighted_cochanges, 1.25);
        assert_eq!(file.sample_commits, vec!["bbbbbbbbbbbb"]);
    }

    #[test]
    fn related_pagerank_hit_conversion_preserves_direct_edge_fields() {
        let edge = CochangeEdge {
            a: "src/a.rs".to_string(),
            b: "src/b.rs".to_string(),
            cochanged_commits: 4,
            weighted_cochanges: 1.25,
            sample_commits: vec!["bbbbbbbbbbbb".to_string()],
        };
        let file = related_file_from_pagerank_hit(
            PageRankHit {
                path: "src/b.rs".to_string(),
                score: 2.0 / 3.0,
            },
            Some(&edge),
        );

        assert_eq!(file.path, "src/b.rs");
        assert_eq!(file.score, 0.667);
        assert_eq!(file.cochanged_commits, 4);
        assert_eq!(file.weighted_cochanges, 1.25);
        assert_eq!(file.sample_commits, vec!["bbbbbbbbbbbb"]);

        let file = related_file_from_pagerank_hit(
            PageRankHit {
                path: "src/indirect.rs".to_string(),
                score: 0.25,
            },
            None,
        );

        assert_eq!(file.path, "src/indirect.rs");
        assert_eq!(file.score, 0.25);
        assert_eq!(file.cochanged_commits, 0);
        assert_eq!(file.weighted_cochanges, 0.0);
        assert!(file.sample_commits.is_empty());
    }

    #[test]
    fn generated_ids_keep_numeric_suffixes_and_do_not_repeat() {
        let ids = (0..1_000).map(|_| new_id("tx")).collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 1_000);
        for id in ids {
            let suffix = id
                .strip_prefix("tx-")
                .expect("generated id should include prefix");
            assert!(
                suffix.bytes().all(|byte| byte.is_ascii_digit()),
                "generated id should keep tx-<digits> format: {id}"
            );
            validate_patch_transaction_id(&id).expect("generated tx id should validate");
        }
    }
}
