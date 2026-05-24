use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
mod related_cli;

use related_cli::{RelatedCli, RelatedCliItem, RelatedCliOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
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
const MAX_SAMPLE_COMMITS: usize = 5;
const MAX_LOG_LINE_BYTES: usize = 64_000;
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
            Self::Cochange => "cochange",
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
            Self::Direct => "direct",
            Self::Pagerank => "pagerank",
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
    pending_utf8: Vec<u8>,
    pending_line_cr: bool,
    saw_bytes: bool,
}

impl FallbackLineSearch {
    fn new(line_number: u64) -> Self {
        Self {
            line_number,
            byte_offset: 0,
            scan_tail: Vec::new(),
            matched: false,
            match_column: 0,
            display_text: String::new(),
            display_char_count: 0,
            display_truncated: false,
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
    commits_scanned: usize,
    commits_matched: usize,
    ignored_large_commits: usize,
    max_commits: usize,
    max_files_per_commit: usize,
    impacted: Vec<ImpactFile>,
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

#[derive(Serialize)]
struct DiffData {
    is_repo: bool,
    summary: String,
    file_count: usize,
    files: Vec<String>,
    omitted_files: usize,
    patch: Option<String>,
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

struct CapturedOutput {
    text: String,
    truncated: bool,
}

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
    let map = build_map(workspace, args.depth, args.include_hidden)?;
    let truncated = map.omitted.any() || map.git.omitted_files();
    let mut summary = format!(
        "{} file(s), languages: {}",
        map.stats.file_count,
        join_or_none(&map.stack.languages)
    );
    if truncated {
        summary.push_str(" (map truncated)");
    }
    let evidence = map_evidence(&map);
    let next_observations = map_next_observations(&map);
    let observation = Observation {
        kind: "workspace_map".to_string(),
        scope: map.root.clone(),
        summary,
        data: map,
        evidence,
        truncated,
        next_observations,
    };

    append_observation_log(workspace, "map", &observation.scope, &observation.summary);
    output_observation(args.json, &observation, print_map)
}

fn cmd_status(workspace: &Workspace, args: JsonArgs) -> Result<()> {
    let git = git_summary(workspace)?;
    let index_status = cochange_index_status(workspace);
    let (recent_operations, recent_operations_omitted, recent_operations_error) =
        match read_log(workspace, 10) {
            Ok(window) => (window.entries, window.omitted_lines, None),
            Err(error) => (Vec::new(), 0, Some(format!("{error:#}"))),
        };
    let recent_operations_truncated = recent_operations_omitted > 0;
    let truncated = git.omitted_files() || recent_operations_truncated;
    let log_note = if recent_operations_error.is_some() {
        ", operation log unreadable"
    } else if recent_operations_truncated {
        ", recent operations truncated"
    } else {
        ""
    };
    let data = StatusData {
        root: workspace.root.to_string_lossy().into_owned(),
        git,
        index_status,
        recent_operations,
        recent_operations_omitted,
        recent_operations_error,
    };
    let mut summary = if data.git.is_repo {
        format!(
            "branch {}, {} dirty file(s), {} untracked file(s), index {}{}",
            data.git.branch.as_deref().unwrap_or("unknown"),
            data.git.dirty_file_count,
            data.git.untracked_file_count,
            data.index_status.status,
            log_note
        )
    } else {
        "not a git repository".to_string()
    };
    if truncated {
        summary.push_str(" (status truncated)");
    }
    let observation = Observation {
        kind: "workspace_status".to_string(),
        scope: data.root.clone(),
        summary,
        data,
        evidence: vec![],
        truncated,
        next_observations: vec![
            "workspace map".to_string(),
            "workspace diff --summary".to_string(),
            "workspace index status".to_string(),
            "workspace log".to_string(),
        ],
    };

    append_observation_log(
        workspace,
        "status",
        &observation.scope,
        &observation.summary,
    );
    output_observation(args.json, &observation, print_status)
}

fn cmd_search(workspace: &Workspace, args: SearchArgs) -> Result<()> {
    let (matches, total_matches, truncated_match_texts) =
        rg_search(workspace, &args.query, args.max_results)?;
    let evidence = matches
        .iter()
        .take(12)
        .map(|m| Evidence {
            path: m.path.clone(),
            lines: Some(m.line.to_string()),
            reason: "text match".to_string(),
        })
        .collect::<Vec<_>>();
    let data = SearchData {
        query: args.query.clone(),
        total_matches,
        truncated_match_texts,
        matches,
    };
    let truncated = data.total_matches > data.matches.len() || data.truncated_match_texts > 0;
    let mut summary = if data.total_matches > data.matches.len() {
        format!(
            "{} match(es) for {:?}, showing {}",
            data.total_matches,
            data.query,
            data.matches.len()
        )
    } else {
        format!("{} match(es) for {:?}", data.total_matches, data.query)
    };
    if data.truncated_match_texts > 0 {
        summary.push_str(&format!(
            ", truncated {} match text(s)",
            data.truncated_match_texts
        ));
    }
    let next_observations = data
        .matches
        .iter()
        .take(5)
        .map(|m| workspace_read_lines_command(&m.path, m.line, m.line))
        .collect();
    let observation = Observation {
        kind: "workspace_search".to_string(),
        scope: workspace.root.to_string_lossy().into_owned(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    };

    append_observation_log(workspace, "search", &args.query, &observation.summary);
    output_observation(args.json, &observation, print_search)
}

fn cmd_index(workspace: &Workspace, args: IndexArgs) -> Result<()> {
    match args.command {
        IndexCommands::Status(args) => cmd_index_status(workspace, args),
        IndexCommands::Cochange(args) => cmd_index_cochange(workspace, args),
    }
}

fn cmd_index_status(workspace: &Workspace, args: IndexStatusArgs) -> Result<()> {
    let data = cochange_index_status(workspace);
    let summary = match data.status.as_str() {
        "fresh" => "co-change index is fresh".to_string(),
        "stale" => "co-change index is stale".to_string(),
        "missing" => "co-change index is missing".to_string(),
        "invalid" => "co-change index is invalid".to_string(),
        "not_git_repo" => "not a git repository".to_string(),
        _ => data.status.clone(),
    };
    let observation = Observation {
        kind: "workspace_index_status".to_string(),
        scope: data.path.clone(),
        summary,
        data,
        evidence: vec![],
        truncated: false,
        next_observations: vec![
            "workspace index cochange".to_string(),
            "workspace related <file> --by cochange --use-index".to_string(),
            "workspace impact --diff --by cochange --use-index".to_string(),
        ],
    };

    append_observation_log(
        workspace,
        "index status",
        &observation.scope,
        &observation.summary,
    );
    output_observation(args.json, &observation, print_index_status)
}

fn cmd_index_cochange(workspace: &Workspace, args: IndexCochangeArgs) -> Result<()> {
    if !workspace.is_git_repo {
        bail!("workspace index cochange requires a git repository");
    }

    ensure_log_writable(workspace)?;
    let index = build_cochange_index(workspace, args.max_commits, args.max_files_per_commit)?;
    let index_path = workspace.cochange_index_path();
    let index_dir = workspace.root.join(INDEX_DIR);
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("failed to create index directory {}", index_dir.display()))?;
    write_cochange_index(&index_path, &index)
        .with_context(|| format!("failed to write index {}", index_path.display()))?;

    let data = IndexCochangeData {
        path: workspace.relative(&index_path),
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
    };
    let summary = format!(
        "indexed {} co-change edge(s) from {} commit(s)",
        data.edge_count, data.commits_indexed
    );
    let observation = Observation {
        kind: "workspace_index_cochange".to_string(),
        scope: data.path.clone(),
        summary,
        data,
        evidence: vec![],
        truncated: false,
        next_observations: vec![
            "workspace related <file> --by cochange --use-index".to_string(),
            "workspace impact --diff --by cochange --use-index".to_string(),
        ],
    };

    append_log(
        workspace,
        "observe",
        "index cochange",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_index_cochange)
}

fn cmd_related(workspace: &Workspace, args: RelatedArgs) -> Result<()> {
    let target = workspace_arg_path(workspace, &args.path)?;
    let data = if workspace.is_git_repo {
        related_by_cochange(
            workspace,
            &target,
            args.max_commits,
            args.max_files_per_commit,
            args.max_results,
            args.rank,
            args.use_index,
        )?
    } else {
        RelatedData {
            target: target.clone(),
            method: args.by.as_str().to_string(),
            ranking: args.rank.as_str().to_string(),
            relationship_source: relationship_source(uses_cochange_index(
                args.use_index,
                args.rank,
            ))
            .to_string(),
            is_repo: false,
            commits_scanned: 0,
            commits_matched: 0,
            ignored_large_commits: 0,
            max_commits: args.max_commits,
            max_files_per_commit: args.max_files_per_commit,
            related: vec![],
        }
    };
    let summary = if data.is_repo {
        format!(
            "{} related file(s) for {} using {} history",
            data.related.len(),
            data.target,
            data.method
        )
    } else {
        "not a git repository".to_string()
    };
    let evidence = related_evidence(&data);
    let next_observations = data
        .related
        .iter()
        .filter(|file| workspace.resolve_path(Path::new(&file.path)).is_file())
        .take(5)
        .map(|file| workspace_read_command(&file.path))
        .collect();
    let observation = Observation {
        kind: "workspace_related".to_string(),
        scope: target.clone(),
        summary,
        data,
        evidence,
        truncated: false,
        next_observations,
    };

    append_observation_log(workspace, "related", &target, &observation.summary);
    output_observation(args.json, &observation, print_related)
}

fn cmd_impact(workspace: &Workspace, args: ImpactArgs) -> Result<()> {
    if !args.diff {
        bail!("workspace impact currently supports only --diff as its source");
    }

    let data = if workspace.is_git_repo {
        impact_by_cochange(
            workspace,
            args.max_commits,
            args.max_files_per_commit,
            args.max_results,
            args.rank,
            args.use_index,
        )?
    } else {
        ImpactData {
            source: "diff".to_string(),
            method: args.by.as_str().to_string(),
            ranking: args.rank.as_str().to_string(),
            relationship_source: relationship_source(uses_cochange_index(
                args.use_index,
                args.rank,
            ))
            .to_string(),
            is_repo: false,
            seed_files: vec![],
            commits_scanned: 0,
            commits_matched: 0,
            ignored_large_commits: 0,
            max_commits: args.max_commits,
            max_files_per_commit: args.max_files_per_commit,
            impacted: vec![],
        }
    };
    let summary = if data.is_repo {
        format!(
            "{} impacted file(s) from {} seed file(s) using {} history",
            data.impacted.len(),
            data.seed_files.len(),
            data.method
        )
    } else {
        "not a git repository".to_string()
    };
    let evidence = impact_evidence(&data);
    let next_observations = data
        .impacted
        .iter()
        .filter(|file| workspace.resolve_path(Path::new(&file.path)).is_file())
        .take(5)
        .map(|file| workspace_read_command(&file.path))
        .collect();
    let observation = Observation {
        kind: "workspace_impact".to_string(),
        scope: data.source.clone(),
        summary,
        data,
        evidence,
        truncated: false,
        next_observations,
    };

    append_observation_log(
        workspace,
        "impact",
        &observation.scope,
        &observation.summary,
    );
    output_observation(args.json, &observation, print_impact)
}

fn cmd_read(workspace: &Workspace, args: ReadArgs) -> Result<()> {
    let path = workspace.resolve_existing_workspace_path(&args.path)?;
    let rel_path = workspace.relative(&path);
    let range = args
        .lines
        .as_deref()
        .map(parse_line_range)
        .transpose()
        .context("invalid --lines range")?;
    let line_label = range.map(|(start, end)| format!("{start}:{end}"));
    let read_content = if let Some((start, end)) = range {
        read_line_range_bounded(&path, start, end)
    } else {
        read_text_prefix_bounded(&path)
    }
    .with_context(|| format!("failed to read text file {}", path.display()))?;

    let data = ReadData {
        path: rel_path.clone(),
        lines: line_label.clone(),
        content: read_content.content,
    };
    let mut summary = match &data.lines {
        Some(lines) => format!("read {} lines {}", data.path, lines),
        None => format!("read {}", data.path),
    };
    if read_content.truncated {
        summary.push_str(" (truncated)");
    }
    let observation = Observation {
        kind: "workspace_read".to_string(),
        scope: data.path.clone(),
        summary,
        data,
        evidence: vec![Evidence {
            path: rel_path.clone(),
            lines: line_label,
            reason: "requested file content".to_string(),
        }],
        truncated: read_content.truncated,
        next_observations: vec![
            format!("workspace search {}", shell_hint(&rel_path)),
            "workspace diff --summary".to_string(),
        ],
    };

    append_observation_log(workspace, "read", &rel_path, &observation.summary);
    output_observation(args.json, &observation, print_read)
}

fn cmd_diff(workspace: &Workspace, args: DiffArgs) -> Result<()> {
    let (data, summary_truncated, patch_truncated) = if workspace.is_git_repo {
        let summary_output =
            git_observable_diff_output_bounded(workspace, ["--stat"], MAX_DIFF_SUMMARY)?;
        let summary_truncated = summary_output.truncated;
        let summary = summary_output.text;
        let diff_files = git_observable_diff_name_only(workspace, MAX_CHANGED_FILES)?;
        let (patch, patch_truncated) = if args.summary {
            (None, false)
        } else {
            let patch = git_observable_diff_output_bounded(workspace, [], MAX_DIFF_PATCH)?;
            (Some(patch.text), patch.truncated)
        };
        (
            DiffData {
                is_repo: true,
                summary,
                file_count: diff_files.total_files,
                files: diff_files.files,
                omitted_files: diff_files.omitted_files,
                patch,
            },
            summary_truncated,
            patch_truncated,
        )
    } else {
        (
            DiffData {
                is_repo: false,
                summary: "not a git repository".to_string(),
                file_count: 0,
                files: vec![],
                omitted_files: 0,
                patch: None,
            },
            false,
            false,
        )
    };
    let file_list_truncated = data.omitted_files > 0;
    let truncated = summary_truncated || patch_truncated || file_list_truncated;
    let mut summary = if data.is_repo {
        format!("{} changed file(s)", data.file_count)
    } else {
        data.summary.clone()
    };
    if summary_truncated && patch_truncated {
        summary.push_str(" (summary and patch truncated)");
    } else if summary_truncated {
        summary.push_str(" (summary truncated)");
    } else if patch_truncated {
        summary.push_str(" (patch truncated)");
    }
    if file_list_truncated {
        summary.push_str(" (files truncated)");
    }
    let evidence = data
        .files
        .iter()
        .map(|path| Evidence {
            path: path.clone(),
            lines: None,
            reason: "git diff changed file".to_string(),
        })
        .collect();
    let next_observations = data
        .files
        .iter()
        .filter(|path| workspace.resolve_path(Path::new(path)).is_file())
        .take(5)
        .map(|path| workspace_read_command(path))
        .collect();
    let observation = Observation {
        kind: "workspace_diff".to_string(),
        scope: workspace.root.to_string_lossy().into_owned(),
        summary,
        data,
        evidence,
        truncated,
        next_observations,
    };

    append_observation_log(workspace, "diff", &observation.scope, &observation.summary);
    output_observation(args.json, &observation, print_diff)
}

fn cmd_patch(workspace: &Workspace, args: PatchArgs) -> Result<()> {
    let patch_path = workspace.resolve_existing_workspace_path(&args.patch_file)?;
    let files_changed = extract_patch_files_from_path(&patch_path)
        .with_context(|| format!("failed to read patch {}", patch_path.display()))?;
    validate_patch_targets(&files_changed)?;
    run_git_apply(workspace, &patch_path, ["--check"])?;
    ensure_log_writable(workspace)?;

    let transaction_id = new_id("tx");
    let transaction_dir = workspace.transaction_dir();
    fs::create_dir_all(&transaction_dir).with_context(|| {
        format!(
            "failed to create transaction directory {}",
            transaction_dir.display()
        )
    })?;
    let stored_patch = transaction_dir.join(format!("{transaction_id}.patch"));
    store_transaction_patch(&patch_path, &stored_patch)?;
    if let Err(error) = run_git_apply(workspace, &patch_path, []) {
        let _ = fs::remove_file(&stored_patch);
        return Err(error);
    }

    let mut observed_files_changed = files_changed.clone();
    let omitted_files = truncate_vec(&mut observed_files_changed, MAX_CHANGED_FILES);
    let data = PatchData {
        transaction_id: transaction_id.clone(),
        patch_file: workspace.relative(&patch_path),
        stored_patch: workspace.relative(&stored_patch),
        file_count: files_changed.len(),
        files_changed: observed_files_changed,
        omitted_files,
    };
    let truncated = data.omitted_files > 0;
    let mut summary = format!(
        "applied patch transaction {} touching {} file(s)",
        data.transaction_id, data.file_count
    );
    if truncated {
        summary.push_str(" (files truncated)");
    }
    let observation = Observation {
        kind: "workspace_patch".to_string(),
        scope: data.patch_file.clone(),
        summary,
        data,
        evidence: files_changed
            .iter()
            .take(MAX_CHANGED_FILES)
            .map(|path| Evidence {
                path: path.clone(),
                lines: None,
                reason: "patch file target".to_string(),
            })
            .collect(),
        truncated,
        next_observations: vec![
            "workspace diff --summary".to_string(),
            format!("workspace rollback {}", transaction_id),
        ],
    };

    append_log(
        workspace,
        "change",
        "patch",
        &observation.scope,
        &args
            .description
            .unwrap_or_else(|| observation.summary.clone()),
        Some(&transaction_id),
    )?;
    output_observation(args.json, &observation, print_patch)
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
    let start = Instant::now();
    let mut command = shell_command(&args.command);
    let mut child = command
        .current_dir(&workspace.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run command {:?}", args.command))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture command stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture command stderr"))?;
    let stdout_reader = std::thread::spawn(move || read_captured_output(stdout));
    let stderr_reader = std::thread::spawn(move || read_captured_output(stderr));
    let status = child
        .wait()
        .with_context(|| format!("failed to wait for command {:?}", args.command))?;
    let duration_ms = start.elapsed().as_millis();
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("stdout reader thread panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("stderr reader thread panicked"))??;
    let truncated = stdout.truncated || stderr.truncated;
    let data = RunData {
        command: args.command.clone(),
        cwd: workspace.root.to_string_lossy().into_owned(),
        exit_code: status.code(),
        duration_ms,
        stdout: stdout.text,
        stderr: stderr.text,
    };
    let mut summary = format!(
        "command exited with {} in {}ms",
        data.exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        data.duration_ms
    );
    if truncated {
        summary.push_str(" (output truncated)");
    }
    let observation = Observation {
        kind: "workspace_run".to_string(),
        scope: data.command.clone(),
        summary,
        data,
        evidence: vec![],
        truncated,
        next_observations: vec![
            "workspace status".to_string(),
            "workspace diff --summary".to_string(),
        ],
    };

    append_log(
        workspace,
        "verify",
        "run",
        &args.command,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_run)
}

fn cmd_log(workspace: &Workspace, args: LogArgs) -> Result<()> {
    let window = read_log(workspace, args.limit)?;
    let data = LogData {
        log_path: workspace.relative(&workspace.log_path()),
        omitted_lines: window.omitted_lines,
        entries: window.entries,
    };
    let truncated = data.omitted_lines > 0;
    let mut summary = format!("{} operation(s)", data.entries.len());
    if truncated {
        summary.push_str(&format!(
            " ({} older log line(s) omitted)",
            data.omitted_lines
        ));
    }
    let observation = Observation {
        kind: "workspace_log".to_string(),
        scope: data.log_path.clone(),
        summary,
        data,
        evidence: vec![],
        truncated,
        next_observations: vec!["workspace status".to_string()],
    };
    output_observation(args.json, &observation, print_log)
}

fn cmd_rollback(workspace: &Workspace, args: RollbackArgs) -> Result<()> {
    let stored_patch = transaction_patch_path(workspace, &args.transaction_id)?;
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

    let rollback_transaction_id = new_id("rb");
    let mut observed_files_changed = files_changed.clone();
    let omitted_files = truncate_vec(&mut observed_files_changed, MAX_CHANGED_FILES);
    let data = RollbackData {
        transaction_id: args.transaction_id.clone(),
        rollback_transaction_id: rollback_transaction_id.clone(),
        stored_patch: workspace.relative(&stored_patch),
        file_count: files_changed.len(),
        files_changed: observed_files_changed,
        omitted_files,
    };
    let truncated = data.omitted_files > 0;
    let mut summary = format!(
        "rolled back transaction {} touching {} file(s)",
        data.transaction_id, data.file_count
    );
    if truncated {
        summary.push_str(" (files truncated)");
    }
    let observation = Observation {
        kind: "workspace_rollback".to_string(),
        scope: data.transaction_id.clone(),
        summary,
        data,
        evidence: files_changed
            .iter()
            .take(MAX_CHANGED_FILES)
            .map(|path| Evidence {
                path: path.clone(),
                lines: None,
                reason: "rollback target".to_string(),
            })
            .collect(),
        truncated,
        next_observations: vec!["workspace diff --summary".to_string()],
    };

    append_log(
        workspace,
        "change",
        "rollback",
        &args.transaction_id,
        &observation.summary,
        Some(&rollback_transaction_id),
    )?;
    output_observation(args.json, &observation, print_rollback)
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
    let mut files = Vec::new();
    let mut directories = BTreeSet::new();
    let mut file_count = 0usize;
    let mut directory_count = 0usize;
    let mut large_file_count = 0usize;
    let mut large_files = Vec::new();
    let mut recent_candidates = Vec::new();

    for entry in WalkDir::new(&workspace.root)
        .max_depth(depth)
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
            directories.insert(rel);
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
        files.push(rel);
    }

    recent_candidates.sort_by_key(|item| std::cmp::Reverse(item.0));
    let recent_files = recent_candidates
        .into_iter()
        .take(12)
        .map(|(_, path)| path)
        .collect::<Vec<_>>();

    let stack = detect_stack(workspace, &files)?;
    let mut structure = detect_structure(&files, directories.into_iter().collect());
    let commands = detect_commands(workspace, &files)?;
    let important_files = important_files(&structure, &stack);
    sort_large_files(&mut large_files);
    let omitted = MapOmittedCounts {
        directories: truncate_vec(&mut structure.directories, MAX_MAP_LIST_ITEMS),
        entrypoints: truncate_vec(&mut structure.entrypoints, MAX_MAP_LIST_ITEMS),
        tests: truncate_vec(&mut structure.tests, MAX_MAP_LIST_ITEMS),
        configs: truncate_vec(&mut structure.configs, MAX_MAP_LIST_ITEMS),
        docs: truncate_vec(&mut structure.docs, MAX_MAP_LIST_ITEMS),
        large_files: large_file_count.saturating_sub(large_files.len()),
    };

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

fn push_recent_candidate(
    recent_candidates: &mut Vec<(SystemTime, String)>,
    modified: SystemTime,
    path: String,
) {
    recent_candidates.push((modified, path));
    recent_candidates.sort_by_key(|item| std::cmp::Reverse(item.0));
    recent_candidates.truncate(MAX_RECENT_FILES);
}

fn push_large_file_candidate(large_files: &mut Vec<LargeFile>, item: LargeFile) {
    large_files.push(item);
    sort_large_files(large_files);
    large_files.truncate(MAX_MAP_LARGE_FILES);
}

fn sort_large_files(large_files: &mut [LargeFile]) {
    large_files.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
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

fn detect_stack(workspace: &Workspace, files: &[String]) -> Result<StackSummary> {
    let file_set = files.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut languages = BTreeSet::new();
    let mut package_managers = BTreeSet::new();
    let mut frameworks = BTreeSet::new();

    for file in files {
        match Path::new(file).extension().and_then(OsStr::to_str) {
            Some("rs") => {
                languages.insert("rust".to_string());
            }
            Some("ts") | Some("tsx") => {
                languages.insert("typescript".to_string());
            }
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => {
                languages.insert("javascript".to_string());
            }
            Some("py") => {
                languages.insert("python".to_string());
            }
            Some("go") => {
                languages.insert("go".to_string());
            }
            Some("java") => {
                languages.insert("java".to_string());
            }
            Some("md") => {
                languages.insert("markdown".to_string());
            }
            _ => {}
        }
    }

    if file_set.contains("Cargo.toml") {
        package_managers.insert("cargo".to_string());
        frameworks.insert("rust-cli".to_string());
    }
    if file_set.contains("package.json") {
        package_managers.insert("npm".to_string());
        let package_json = workspace.root.join("package.json");
        if let Ok(detected_frameworks) = detect_package_json_frameworks(&package_json) {
            for framework in detected_frameworks {
                frameworks.insert(framework);
            }
        }
    }
    if file_set.contains("pnpm-lock.yaml") {
        package_managers.insert("pnpm".to_string());
    }
    if file_set.contains("yarn.lock") {
        package_managers.insert("yarn".to_string());
    }
    if file_set.contains("go.mod") {
        package_managers.insert("go".to_string());
    }
    if file_set.contains("pyproject.toml") {
        package_managers.insert("python/pyproject".to_string());
    }
    if file_set.contains("requirements.txt") {
        package_managers.insert("pip".to_string());
    }

    Ok(StackSummary {
        languages: languages.into_iter().collect(),
        package_managers: package_managers.into_iter().collect(),
        frameworks: frameworks.into_iter().collect(),
    })
}

fn detect_structure(files: &[String], directories: Vec<String>) -> StructureSummary {
    let file_set = files.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let entrypoint_names = [
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
    let config_names = [
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

    let entrypoints = entrypoint_names
        .iter()
        .filter(|path| file_set.contains(**path))
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    let mut tests = files
        .iter()
        .filter(|path| is_test_file(path))
        .cloned()
        .collect::<Vec<_>>();
    tests.sort();
    let mut configs = config_names
        .iter()
        .filter(|path| file_set.contains(**path))
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    let mut config_extras = files
        .iter()
        .filter(|path| path.ends_with(".config.js") && !config_names.contains(&path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    config_extras.sort();
    configs.extend(config_extras);
    let mut docs = files
        .iter()
        .filter(|path| {
            let lower = path.to_lowercase();
            lower == "readme.md" || lower.starts_with("docs/") || lower.ends_with(".md")
        })
        .cloned()
        .collect::<Vec<_>>();
    docs.sort();
    let mut directories = directories;
    directories.sort();

    StructureSummary {
        directories,
        entrypoints,
        tests,
        configs,
        docs,
    }
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

        let mut scan = Vec::with_capacity(tail.len() + bytes_read);
        scan.extend_from_slice(&tail);
        scan.extend_from_slice(&buffer[..bytes_read]);
        for (index, (needle, _)) in needles.iter().enumerate() {
            if !matched[index] && scan.windows(needle.len()).any(|window| window == *needle) {
                matched[index] = true;
            }
        }
        if max_needle_len > 1 {
            let tail_len = (max_needle_len - 1).min(scan.len());
            tail = scan[scan.len() - tail_len..].to_vec();
        }
    }

    Ok(matched)
}

fn read_json_file(path: &Path) -> Result<Value> {
    let file = fs::File::open(path)?;
    serde_json::from_reader(BufReader::new(file)).context("failed to parse JSON")
}

fn detect_commands(workspace: &Workspace, files: &[String]) -> Result<BTreeMap<String, String>> {
    let mut commands = BTreeMap::new();
    let file_set = files.iter().map(String::as_str).collect::<BTreeSet<_>>();

    if file_set.contains("Cargo.toml") {
        commands.insert("build".to_string(), "cargo build".to_string());
        commands.insert("test".to_string(), "cargo test".to_string());
        commands.insert("run".to_string(), "cargo run --".to_string());
    }

    if file_set.contains("package.json") {
        let package_json = workspace.root.join("package.json");
        if let Ok(value) = read_json_file(&package_json)
            && let Some(scripts) = value.get("scripts").and_then(Value::as_object)
        {
            for (name, value) in scripts {
                if let Some(script) = value.as_str() {
                    commands.insert(name.clone(), format!("npm run {name} # {script}"));
                }
            }
        }
    }

    if file_set.contains("Makefile") {
        commands
            .entry("make".to_string())
            .or_insert("make".to_string());
    }
    if file_set.contains("justfile") {
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
            reason: "configuration or package manifest".to_string(),
        });
    }
    for path in &structure.entrypoints {
        items.push(ImportantFile {
            path: path.clone(),
            reason: "likely entrypoint".to_string(),
        });
    }
    if let Some(doc) = structure
        .docs
        .iter()
        .find(|path| path.eq_ignore_ascii_case("README.md"))
    {
        items.push(ImportantFile {
            path: doc.clone(),
            reason: "primary project documentation".to_string(),
        });
    }
    if stack.languages.is_empty() {
        items.push(ImportantFile {
            path: ".".to_string(),
            reason: "no language signals detected yet".to_string(),
        });
    }
    items
}

fn map_evidence(map: &WorkspaceMap) -> Vec<Evidence> {
    map.important_files
        .iter()
        .take(16)
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
    for file in map.important_files.iter().take(4) {
        if file.path != "README.md" && file.path != "." {
            next.push(workspace_read_command(&file.path));
        }
    }
    if map.git.is_repo {
        next.push("workspace diff --summary".to_string());
        next.push("workspace index status".to_string());
        next.push("workspace index cochange".to_string());
        next.push("workspace impact --diff --by cochange".to_string());
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
    if !uses_cochange_index(use_index, rank)
        && let Some(cli) = RelatedCli::detect()
    {
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

    if uses_cochange_index(use_index, rank) {
        let index = read_cochange_index(workspace)?;
        let ranking = match rank {
            RankingMethod::Direct => rank_cochanges_from_index(&index, target, max_results),
            RankingMethod::Pagerank => {
                rank_cochanges_pagerank_from_index(&index, target, max_results)
            }
        };
        return Ok(RelatedData {
            target: target.to_string(),
            method: "cochange".to_string(),
            ranking: rank.as_str().to_string(),
            relationship_source: "cochange-index".to_string(),
            is_repo: true,
            commits_scanned: index.commits_scanned,
            commits_matched: ranking.commits_matched,
            ignored_large_commits: index.ignored_large_commits,
            max_commits: index.max_commits,
            max_files_per_commit: index.max_files_per_commit,
            related: ranking.related,
        });
    }

    let commits = git_recent_name_only_commits(workspace, max_commits)?;
    let ranking = rank_cochanges(&commits, target, max_files_per_commit, max_results);
    Ok(RelatedData {
        target: target.to_string(),
        method: "cochange".to_string(),
        ranking: rank.as_str().to_string(),
        relationship_source: "git-log".to_string(),
        is_repo: true,
        commits_scanned: commits.len(),
        commits_matched: ranking.commits_matched,
        ignored_large_commits: ranking.ignored_large_commits,
        max_commits,
        max_files_per_commit,
        related: ranking.related,
    })
}

fn related_data_from_related_cli(
    target: &str,
    output: RelatedCliOutput,
    max_commits: usize,
    max_files_per_commit: usize,
    max_results: usize,
    rank: RankingMethod,
) -> RelatedData {
    let mut related = output
        .related
        .into_iter()
        .filter_map(related_file_from_related_cli)
        .collect::<Vec<_>>();
    related.truncate(max_results);
    let commits_matched = related
        .iter()
        .map(|item| item.cochanged_commits)
        .max()
        .unwrap_or(0);
    RelatedData {
        target: target.to_string(),
        method: "cochange".to_string(),
        ranking: rank.as_str().to_string(),
        relationship_source: format!("related-cli:{}", output.mode),
        is_repo: true,
        commits_scanned: 0,
        commits_matched,
        ignored_large_commits: 0,
        max_commits,
        max_files_per_commit,
        related,
    }
}

fn related_file_from_related_cli(item: RelatedCliItem) -> Option<RelatedFile> {
    let path = normalize_repo_path(&item.path);
    should_include_repo_file(&path).then(|| RelatedFile {
        path,
        score: round3(item.score),
        cochanged_commits: item.cochanges,
        weighted_cochanges: round3(item.weight),
        sample_commits: item
            .evidence
            .iter()
            .take(MAX_SAMPLE_COMMITS)
            .map(|evidence| short_commit(&evidence.hash).to_string())
            .collect(),
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
        let files = commit
            .files
            .iter()
            .map(|file| normalize_repo_path(file))
            .filter(|file| should_include_repo_file(file))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        if files.len() > max_files_per_commit.max(1) {
            ignored_large_commits += 1;
            continue;
        }
        if files.len() < 2 {
            continue;
        }

        commits_indexed += 1;
        for file in &files {
            *file_commit_counts.entry(file.clone()).or_default() += 1;
        }

        let file_count = files.len().max(2);
        let recency_weight = 1.0 / (1.0 + rank as f64 / 50.0);
        let size_weight = 1.0 / (file_count as f64 + 1.0).ln();
        let weight = recency_weight * size_weight;

        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                let key = (files[i].clone(), files[j].clone());
                let accumulator = accumulators.entry(key).or_default();
                accumulator.cochanged_commits += 1;
                accumulator.weighted_cochanges += weight;
                if accumulator.sample_commits.len() < 5 {
                    accumulator.sample_commits.push(short_commit(&commit.hash));
                }
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

fn cochange_index_status(workspace: &Workspace) -> IndexStatusData {
    let path = workspace.cochange_index_path();
    let path_label = workspace.relative(&path);
    if !workspace.is_git_repo {
        return empty_index_status(false, path_label, "not_git_repo", false, false, None, None);
    }

    let current_head = git_current_head(workspace).ok().flatten();
    if !path.exists() {
        return empty_index_status(
            true,
            path_label,
            "missing",
            false,
            false,
            current_head,
            None,
        );
    }

    match read_cochange_index(workspace) {
        Ok(index) => {
            let fresh = current_head.is_some() && current_head == index.head;
            IndexStatusData {
                is_repo: true,
                path: path_label,
                exists: true,
                readable: true,
                status: if fresh { "fresh" } else { "stale" }.to_string(),
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
        Err(error) => empty_index_status(
            true,
            path_label,
            "invalid",
            true,
            false,
            current_head,
            Some(error.to_string()),
        ),
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
    if !uses_cochange_index(use_index, rank)
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

    if uses_cochange_index(use_index, rank) {
        let index = read_cochange_index(workspace)?;
        let ranking = match rank {
            RankingMethod::Direct => {
                rank_cochange_impact_from_index(&index, &seed_files, max_results)
            }
            RankingMethod::Pagerank => {
                rank_cochange_impact_pagerank_from_index(&index, &seed_files, max_results)
            }
        };
        return Ok(ImpactData {
            source: "diff".to_string(),
            method: "cochange".to_string(),
            ranking: rank.as_str().to_string(),
            relationship_source: "cochange-index".to_string(),
            is_repo: true,
            seed_files,
            commits_scanned: index.commits_scanned,
            commits_matched: ranking.commits_matched,
            ignored_large_commits: index.ignored_large_commits,
            max_commits: index.max_commits,
            max_files_per_commit: index.max_files_per_commit,
            impacted: ranking.impacted,
        });
    }

    let commits = git_recent_name_only_commits(workspace, max_commits)?;
    let ranking = rank_cochange_impact(&commits, &seed_files, max_files_per_commit, max_results);

    Ok(ImpactData {
        source: "diff".to_string(),
        method: "cochange".to_string(),
        ranking: rank.as_str().to_string(),
        relationship_source: "git-log".to_string(),
        is_repo: true,
        seed_files,
        commits_scanned: commits.len(),
        commits_matched: ranking.commits_matched,
        ignored_large_commits: ranking.ignored_large_commits,
        max_commits,
        max_files_per_commit,
        impacted: ranking.impacted,
    })
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

    let seed_set = seed_files.iter().cloned().collect::<BTreeSet<_>>();
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
            for evidence in item.evidence {
                if accumulator.sample_commits.len() >= 5 {
                    break;
                }
                accumulator
                    .sample_commits
                    .push(short_commit(&evidence.hash).to_string());
            }
        }
    }

    let max_score = accumulators
        .values()
        .map(|item| item.score)
        .fold(0.0, f64::max);
    let mut impacted = accumulators
        .into_iter()
        .map(|(path, item)| ImpactFile {
            path,
            score: if max_score > 0.0 {
                round3(item.score / max_score)
            } else {
                0.0
            },
            cochanged_commits: item.cochanged_commits,
            weighted_cochanges: round3(item.weighted_cochanges),
            seed_files: item.seed_files.into_iter().collect(),
            sample_commits: item.sample_commits,
        })
        .collect::<Vec<_>>();
    impacted.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
    impacted.truncate(max_results);

    Ok(Some(ImpactData {
        source: "diff".to_string(),
        method: "cochange".to_string(),
        ranking: rank.as_str().to_string(),
        relationship_source: format!("related-cli:{}:aggregate", rank.as_str()),
        is_repo: true,
        seed_files: seed_files.to_vec(),
        commits_scanned: 0,
        commits_matched: impacted
            .iter()
            .map(|item| item.cochanged_commits)
            .max()
            .unwrap_or(0),
        ignored_large_commits: 0,
        max_commits,
        max_files_per_commit,
        impacted,
    }))
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git log stderr"))?;
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));

    let commits_result = read_git_log_name_only(stdout);
    let status = child.wait().context("failed to wait for git log")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git log stderr reader thread panicked"))??;
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
    for path in git_output_name_only(workspace, args)? {
        if should_include_repo_file(&path) {
            files.insert(path);
        }
    }
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

fn git_output_name_only<I, S>(workspace: &Workspace, args: I) -> Result<Vec<String>>
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git stderr"))?;
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));

    let paths_result = read_git_name_only_paths(stdout);
    let status = child.wait().context("failed to wait for git")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git stderr reader thread panicked"))??;
    let paths = paths_result?;
    if !status.success() {
        bail!("git failed: {}", stderr.text.trim());
    }
    Ok(paths)
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git stderr"))?;
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));

    let paths_result = read_git_name_only_paths_limited(stdout, max_files);
    let status = child.wait().context("failed to wait for git")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git stderr reader thread panicked"))??;
    let paths = paths_result?;
    if !status.success() {
        bail!("git failed: {}", stderr.text.trim());
    }
    Ok(paths)
}

fn read_git_name_only_paths<R: Read>(reader: R) -> Result<Vec<String>> {
    read_git_name_only_paths_limited(reader, usize::MAX).map(|paths| paths.files)
}

fn read_git_name_only_paths_limited<R: Read>(
    reader: R,
    max_files: usize,
) -> Result<BoundedFileList> {
    let mut reader = BufReader::new(reader);
    let mut line_number = 1usize;
    let mut files = Vec::new();
    let mut total_files = 0usize;

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
            total_files += 1;
            if files.len() < max_files {
                files.push(path);
            }
        }
    }

    Ok(BoundedFileList {
        omitted_files: total_files.saturating_sub(files.len()),
        files,
        total_files,
    })
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
        let files = commit
            .files
            .iter()
            .map(|file| normalize_repo_path(file))
            .filter(|file| !file.is_empty())
            .collect::<BTreeSet<_>>();

        if !files.contains(&target) {
            continue;
        }
        commits_matched += 1;

        if files.len() > max_files_per_commit.max(1) {
            ignored_large_commits += 1;
            continue;
        }

        let file_count = files.len().max(2);
        let recency_weight = 1.0 / (1.0 + rank as f64 / 50.0);
        let size_weight = 1.0 / (file_count as f64 + 1.0).ln();
        let weight = recency_weight * size_weight;

        for file in files {
            if file == target {
                continue;
            }
            let accumulator = accumulators.entry(file).or_default();
            accumulator.cochanged_commits += 1;
            accumulator.weighted_cochanges += weight;
            if accumulator.sample_commits.len() < 5 {
                accumulator.sample_commits.push(short_commit(&commit.hash));
            }
        }
    }

    let max_weight = accumulators
        .values()
        .map(|item| item.weighted_cochanges)
        .fold(0.0, f64::max);
    let mut related = accumulators
        .into_iter()
        .map(|(path, item)| RelatedFile {
            path,
            score: if max_weight > 0.0 {
                round3(item.weighted_cochanges / max_weight)
            } else {
                0.0
            },
            cochanged_commits: item.cochanged_commits,
            weighted_cochanges: round3(item.weighted_cochanges),
            sample_commits: item.sample_commits,
        })
        .collect::<Vec<_>>();

    related.sort_by(|a, b| {
        b.weighted_cochanges
            .total_cmp(&a.weighted_cochanges)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
    related.truncate(max_results);

    CochangeRanking {
        related,
        commits_matched,
        ignored_large_commits,
    }
}

fn rank_cochanges_from_index(
    index: &CochangeIndex,
    target: &str,
    max_results: usize,
) -> CochangeRanking {
    let target = normalize_repo_path(target);
    let mut related = index
        .edges
        .iter()
        .filter_map(|edge| {
            let path = if edge.a == target {
                edge.b.clone()
            } else if edge.b == target {
                edge.a.clone()
            } else {
                return None;
            };

            Some(RelatedFile {
                path,
                score: 0.0,
                cochanged_commits: edge.cochanged_commits,
                weighted_cochanges: edge.weighted_cochanges,
                sample_commits: edge.sample_commits.clone(),
            })
        })
        .collect::<Vec<_>>();

    normalize_related_scores(&mut related);
    related.truncate(max_results);

    CochangeRanking {
        related,
        commits_matched: index.file_commit_counts.get(&target).copied().unwrap_or(0),
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
    let mut related = hits
        .into_iter()
        .map(|hit| {
            let direct_edge = find_cochange_edge(index, &target, &hit.path);
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
        })
        .collect::<Vec<_>>();

    related.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
    related.truncate(max_results);

    CochangeRanking {
        related,
        commits_matched: index.file_commit_counts.get(&target).copied().unwrap_or(0),
        ignored_large_commits: 0,
    }
}

fn rank_cochange_impact(
    commits: &[GitCommitFiles],
    seed_files: &[String],
    max_files_per_commit: usize,
    max_results: usize,
) -> ImpactRanking {
    let seed_files = seed_files
        .iter()
        .map(|file| normalize_repo_path(file))
        .filter(|file| !file.is_empty())
        .collect::<BTreeSet<_>>();
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
        let files = commit
            .files
            .iter()
            .map(|file| normalize_repo_path(file))
            .filter(|file| should_include_repo_file(file))
            .collect::<BTreeSet<_>>();
        let matched_seeds = files.intersection(&seed_files).cloned().collect::<Vec<_>>();

        if matched_seeds.is_empty() {
            continue;
        }
        commits_matched += 1;

        if files.len() > max_files_per_commit.max(1) {
            ignored_large_commits += 1;
            continue;
        }

        let file_count = files.len().max(2);
        let recency_weight = 1.0 / (1.0 + rank as f64 / 50.0);
        let size_weight = 1.0 / (file_count as f64 + 1.0).ln();
        let seed_weight = 1.0 + (matched_seeds.len().saturating_sub(1) as f64 * 0.25);
        let weight = recency_weight * size_weight * seed_weight;

        for file in files {
            if seed_files.contains(&file) {
                continue;
            }
            let accumulator = accumulators.entry(file).or_default();
            accumulator.cochanged_commits += 1;
            accumulator.weighted_cochanges += weight;
            accumulator.seed_files.extend(matched_seeds.iter().cloned());
            if accumulator.sample_commits.len() < 5 {
                accumulator.sample_commits.push(short_commit(&commit.hash));
            }
        }
    }

    let max_weight = accumulators
        .values()
        .map(|item| item.weighted_cochanges)
        .fold(0.0, f64::max);
    let mut impacted = accumulators
        .into_iter()
        .map(|(path, item)| ImpactFile {
            path,
            score: if max_weight > 0.0 {
                round3(item.weighted_cochanges / max_weight)
            } else {
                0.0
            },
            cochanged_commits: item.cochanged_commits,
            weighted_cochanges: round3(item.weighted_cochanges),
            seed_files: item.seed_files.into_iter().collect(),
            sample_commits: item.sample_commits,
        })
        .collect::<Vec<_>>();

    impacted.sort_by(|a, b| {
        b.weighted_cochanges
            .total_cmp(&a.weighted_cochanges)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
    impacted.truncate(max_results);

    ImpactRanking {
        impacted,
        commits_matched,
        ignored_large_commits,
    }
}

fn rank_cochange_impact_from_index(
    index: &CochangeIndex,
    seed_files: &[String],
    max_results: usize,
) -> ImpactRanking {
    let seed_files = seed_files
        .iter()
        .map(|file| normalize_repo_path(file))
        .filter(|file| !file.is_empty())
        .collect::<BTreeSet<_>>();
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
            if accumulator.sample_commits.len() >= 5 {
                break;
            }
            if !accumulator.sample_commits.contains(commit) {
                accumulator.sample_commits.push(commit.clone());
            }
        }
    }

    let mut impacted = accumulators
        .into_iter()
        .map(|(path, item)| ImpactFile {
            path,
            score: 0.0,
            cochanged_commits: item.cochanged_commits,
            weighted_cochanges: round3(item.weighted_cochanges),
            seed_files: item.seed_files.into_iter().collect(),
            sample_commits: item.sample_commits,
        })
        .collect::<Vec<_>>();

    normalize_impact_scores(&mut impacted);
    impacted.truncate(max_results);
    let commits_matched = seed_files
        .iter()
        .filter_map(|file| index.file_commit_counts.get(file))
        .sum();

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
    let seed_files = seed_files
        .iter()
        .map(|file| normalize_repo_path(file))
        .filter(|file| !file.is_empty())
        .collect::<BTreeSet<_>>();
    let hits = personalized_pagerank(index, &seed_files, 40, 0.85);
    let mut impacted = hits
        .into_iter()
        .map(|hit| {
            let mut direct_commits = 0usize;
            let mut direct_weight = 0.0f64;
            let mut direct_seeds = BTreeSet::new();
            let mut sample_commits = Vec::new();

            for seed in &seed_files {
                if let Some(edge) = find_cochange_edge(index, seed, &hit.path) {
                    direct_commits += edge.cochanged_commits;
                    direct_weight += edge.weighted_cochanges;
                    direct_seeds.insert(seed.clone());
                    for commit in &edge.sample_commits {
                        if sample_commits.len() >= 5 {
                            break;
                        }
                        if !sample_commits.contains(commit) {
                            sample_commits.push(commit.clone());
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
        })
        .collect::<Vec<_>>();

    impacted.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
    impacted.truncate(max_results);
    let commits_matched = seed_files
        .iter()
        .filter_map(|file| index.file_commit_counts.get(file))
        .sum();

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

    let active_seeds = seed_files
        .iter()
        .filter(|seed| graph.contains_key(*seed))
        .cloned()
        .collect::<Vec<_>>();
    if active_seeds.is_empty() {
        return vec![];
    }

    let seed_probability = 1.0 / active_seeds.len() as f64;
    let mut personalization = BTreeMap::<String, f64>::new();
    for seed in &active_seeds {
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

            let total_weight = neighbors.iter().map(|(_, weight)| *weight).sum::<f64>();
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

    let max_score = rank
        .iter()
        .filter(|(path, _)| !seed_files.contains(*path))
        .map(|(_, score)| *score)
        .fold(0.0, f64::max);
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
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
    });
    hits
}

fn find_cochange_edge<'a>(index: &'a CochangeIndex, a: &str, b: &str) -> Option<&'a CochangeEdge> {
    let a = normalize_repo_path(a);
    let b = normalize_repo_path(b);
    index
        .edges
        .iter()
        .find(|edge| (edge.a == a && edge.b == b) || (edge.a == b && edge.b == a))
}

fn normalize_related_scores(related: &mut [RelatedFile]) {
    let max_weight = related
        .iter()
        .map(|item| item.weighted_cochanges)
        .fold(0.0, f64::max);
    for item in related.iter_mut() {
        item.score = if max_weight > 0.0 {
            round3(item.weighted_cochanges / max_weight)
        } else {
            0.0
        };
    }
    related.sort_by(|a, b| {
        b.weighted_cochanges
            .total_cmp(&a.weighted_cochanges)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn normalize_impact_scores(impacted: &mut [ImpactFile]) {
    let max_weight = impacted
        .iter()
        .map(|item| item.weighted_cochanges)
        .fold(0.0, f64::max);
    for item in impacted.iter_mut() {
        item.score = if max_weight > 0.0 {
            round3(item.weighted_cochanges / max_weight)
        } else {
            0.0
        };
    }
    impacted.sort_by(|a, b| {
        b.weighted_cochanges
            .total_cmp(&a.weighted_cochanges)
            .then_with(|| b.cochanged_commits.cmp(&a.cochanged_commits))
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn related_evidence(data: &RelatedData) -> Vec<Evidence> {
    data.related
        .iter()
        .take(12)
        .map(|file| Evidence {
            path: file.path.clone(),
            lines: None,
            reason: if data.ranking == "pagerank" && file.cochanged_commits == 0 {
                format!(
                    "reached from {} through the co-change graph; pagerank score {:.3}",
                    data.target, file.score
                )
            } else {
                format!(
                    "changed with {} in {} commit(s); samples: {}",
                    data.target,
                    file.cochanged_commits,
                    join_or_none(&file.sample_commits)
                )
            },
        })
        .collect()
}

fn impact_evidence(data: &ImpactData) -> Vec<Evidence> {
    data.impacted
        .iter()
        .take(12)
        .map(|file| Evidence {
            path: file.path.clone(),
            lines: None,
            reason: if data.ranking == "pagerank" && file.cochanged_commits == 0 {
                format!(
                    "reached from seed file(s) {} through the co-change graph; pagerank score {:.3}",
                    join_or_none(&file.seed_files),
                    file.score
                )
            } else {
                format!(
                    "changed with seed file(s) {} in {} commit(s); samples: {}",
                    join_or_none(&file.seed_files),
                    file.cochanged_commits,
                    join_or_none(&file.sample_commits)
                )
            },
        })
        .collect()
}

fn relationship_source(use_index: bool) -> &'static str {
    if use_index {
        "cochange-index"
    } else {
        "git-log"
    }
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
    let mut dirty_files = Vec::new();
    let mut untracked_files = Vec::new();
    let mut dirty_file_count = 0usize;
    let mut untracked_file_count = 0usize;

    stream_git_status_entries(workspace, |code, path| {
        if path == LOG_DIR || path.starts_with(&format!("{LOG_DIR}/")) {
            return;
        }
        if code == "??" {
            untracked_file_count += 1;
            if untracked_files.len() < MAX_GIT_STATUS_FILES {
                untracked_files.push(path);
            }
        } else {
            dirty_file_count += 1;
            if dirty_files.len() < MAX_GIT_STATUS_FILES {
                dirty_files.push(path);
            }
        }
    })?;

    Ok(GitSummary {
        is_repo: true,
        branch,
        dirty_file_count,
        untracked_file_count,
        omitted_dirty_files: dirty_file_count.saturating_sub(dirty_files.len()),
        omitted_untracked_files: untracked_file_count.saturating_sub(untracked_files.len()),
        dirty_files,
        untracked_files,
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git status stderr"))?;
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));

    let read_result = read_git_status_stdout(stdout, &mut handle);
    let status = child.wait().context("failed to wait for git status")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git status stderr reader thread panicked"))??;
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture ripgrep stderr"))?;
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));
    let search_result = parse_rg_json_output(stdout, max_results);
    let status = child.wait().context("failed to wait for ripgrep")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("ripgrep stderr reader thread panicked"))??;

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
    let mut file_paths = Vec::new();

    for entry in WalkDir::new(&workspace.root)
        .into_iter()
        .filter_entry(|entry| entry.path() == workspace.root || should_descend(entry.path(), false))
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        file_paths.push(entry.into_path());
    }
    file_paths.sort();

    for path in file_paths {
        let rel_path = workspace.relative(&path);
        if !should_include_repo_file(&rel_path) {
            continue;
        }

        let remaining_results = max_results.saturating_sub(matches.len());
        let Ok(file_result) = fallback_text_search_file(&path, &rel_path, query, remaining_results)
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
    let mut line = FallbackLineSearch::new(1);

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
            line = FallbackLineSearch::new(line.line_number + 1);
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

    let mut scan = Vec::with_capacity(line.scan_tail.len() + bytes.len());
    scan.extend_from_slice(&line.scan_tail);
    scan.extend_from_slice(bytes);
    if let Some(index) = scan.windows(query.len()).position(|window| window == query) {
        line.matched = true;
        line.match_column =
            line.byte_offset.saturating_sub(line.scan_tail.len()) as u64 + index as u64 + 1;
    }

    let tail_len = query.len().saturating_sub(1).min(scan.len());
    line.scan_tail = scan[scan.len() - tail_len..].to_vec();
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

    if !line.display_truncated {
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
            output.push_str("\n[output truncated]\n");
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
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git apply")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture git apply stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git apply stderr"))?;
    let stdout_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stdout, MAX_CAPTURED_OUTPUT));
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));
    let status = child.wait().context("failed to wait for git apply")?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("git apply stdout reader thread panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git apply stderr reader thread panicked"))??;
    if !status.success() {
        let message = if stderr.text.trim().is_empty() {
            stdout.text.trim()
        } else {
            stderr.text.trim()
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
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture git stderr"))?;
    let stdout_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stdout, max_stdout_bytes));
    let stderr_reader =
        std::thread::spawn(move || read_captured_output_with_limit(stderr, MAX_CAPTURED_OUTPUT));
    let status = child.wait().context("failed to wait for git")?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("git stdout reader thread panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("git stderr reader thread panicked"))??;
    if !status.success() {
        bail!("git failed: {}", stderr.text.trim());
    }
    Ok(stdout)
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

fn read_captured_output<R: Read>(mut reader: R) -> Result<CapturedOutput> {
    read_captured_output_with_limit(&mut reader, MAX_CAPTURED_OUTPUT)
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
        text.push_str("\n[output truncated]\n");
    }
    Ok(CapturedOutput { text, truncated })
}

fn append_log(
    workspace: &Workspace,
    kind: &str,
    op: &str,
    scope: &str,
    summary: &str,
    transaction_id: Option<&str>,
) -> Result<()> {
    let log_dir = workspace.root.join(LOG_DIR);
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log directory {}", log_dir.display()))?;

    let entry = LogEntry {
        id: new_id("op"),
        timestamp_unix_ms: now_ms(),
        kind: kind.to_string(),
        op: op.to_string(),
        scope: truncate_inline(scope, MAX_LOG_SCOPE),
        summary: truncate_inline(summary, MAX_LOG_SUMMARY),
        transaction_id: transaction_id.map(ToOwned::to_owned),
    };
    let line = serde_json::to_string(&entry)?;
    use std::io::Write;
    let mut file = open_log_for_append(workspace)?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn append_observation_log(workspace: &Workspace, op: &str, scope: &str, summary: &str) {
    let _ = append_log(workspace, "observe", op, scope, summary, None);
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
            "not a git repository".to_string()
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
        if data.git.omitted_dirty_files > 0 {
            println!(
                "    ... {} more dirty file(s)",
                data.git.omitted_dirty_files
            );
        }
        print_list("untracked", &data.git.untracked_files);
        if data.git.omitted_untracked_files > 0 {
            println!(
                "    ... {} more untracked file(s)",
                data.git.omitted_untracked_files
            );
        }
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
        "  scanned: {} commit(s), matched: {}, ignored broad commits: {}",
        data.commits_scanned, data.commits_matched, data.ignored_large_commits
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
    println!(
        "  scanned: {} commit(s), matched: {}, ignored broad commits: {}",
        data.commits_scanned, data.commits_matched, data.ignored_large_commits
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

fn print_read(observation: &Observation<ReadData>) -> Result<()> {
    print!("{}", observation.data.content);
    if !observation.data.content.ends_with('\n') {
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
    if !data.summary.trim().is_empty() {
        println!("{}", data.summary.trim_end());
    }
    if let Some(patch) = &data.patch
        && !patch.trim().is_empty()
    {
        println!("{patch}");
    }
    Ok(())
}

fn print_patch(observation: &Observation<PatchData>) -> Result<()> {
    println!("{}", observation.summary);
    println!("  transaction: {}", observation.data.transaction_id);
    print_list("files", &observation.data.files_changed);
    if observation.data.omitted_files > 0 {
        println!("    ... {} more file(s)", observation.data.omitted_files);
    }
    Ok(())
}

fn print_run(observation: &Observation<RunData>) -> Result<()> {
    let data = &observation.data;
    if !data.stdout.is_empty() {
        print!("{}", data.stdout);
        if !data.stdout.ends_with('\n') {
            println!();
        }
    }
    if !data.stderr.is_empty() {
        eprint!("{}", data.stderr);
        if !data.stderr.ends_with('\n') {
            eprintln!();
        }
    }
    println!("{}", observation.summary);
    Ok(())
}

fn print_log(observation: &Observation<LogData>) -> Result<()> {
    if observation.data.entries.is_empty() {
        println!("no operations recorded");
        return Ok(());
    }
    for entry in &observation.data.entries {
        println!(
            "{} {} {} {} - {}",
            entry.timestamp_unix_ms, entry.kind, entry.op, entry.scope, entry.summary
        );
    }
    if observation.data.omitted_lines > 0 {
        println!(
            "... {} older log line(s) omitted",
            observation.data.omitted_lines
        );
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
    if observation.data.omitted_files > 0 {
        println!("    ... {} more file(s)", observation.data.omitted_files);
    }
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
    truncated.push_str("\n[output truncated]\n");
    truncated
}

fn truncate_inline(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(" [truncated]");
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
        fs::write(temp.path().join("a.txt"), "needle one\nneedle two\n")
            .expect("file should be written");
        fs::write(temp.path().join("b.txt"), "needle three\n").expect("file should be written");
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
        let structure = detect_structure(&files, vec!["z".to_string(), "a".to_string()]);

        assert_eq!(structure.directories, vec!["a", "z"]);
        assert_eq!(structure.entrypoints, vec!["src/main.rs", "index.js"]);
        assert_eq!(structure.configs, vec!["Cargo.toml", "vite.config.js"]);
        assert_eq!(structure.tests, vec!["tests/a_test.rs", "tests/z_test.rs"]);
        assert_eq!(structure.docs, vec!["README.md", "docs/guide.md"]);
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
