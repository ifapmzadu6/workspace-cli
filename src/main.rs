use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
mod related_cli;

use related_cli::{RelatedCli, RelatedCliItem, RelatedCliOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
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
    dirty_files: Vec<String>,
    untracked_files: Vec<String>,
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
}

#[derive(Serialize)]
struct SearchData {
    query: String,
    matches: Vec<SearchMatch>,
}

#[derive(Serialize)]
struct SearchMatch {
    path: String,
    line: u64,
    column: u64,
    text: String,
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

#[derive(Serialize)]
struct DiffData {
    is_repo: bool,
    summary: String,
    files: Vec<String>,
    patch: Option<String>,
}

#[derive(Serialize)]
struct PatchData {
    transaction_id: String,
    patch_file: String,
    stored_patch: String,
    files_changed: Vec<String>,
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
    entries: Vec<LogEntry>,
}

#[derive(Serialize)]
struct RollbackData {
    transaction_id: String,
    rollback_transaction_id: String,
    stored_patch: String,
    files_changed: Vec<String>,
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
    let summary = format!(
        "{} file(s), languages: {}",
        map.stats.file_count,
        join_or_none(&map.stack.languages)
    );
    let evidence = map_evidence(&map);
    let next_observations = map_next_observations(&map);
    let observation = Observation {
        kind: "workspace_map".to_string(),
        scope: map.root.clone(),
        summary,
        data: map,
        evidence,
        truncated: false,
        next_observations,
    };

    append_log(
        workspace,
        "observe",
        "map",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_map)
}

fn cmd_status(workspace: &Workspace, args: JsonArgs) -> Result<()> {
    let git = git_summary(workspace)?;
    let index_status = cochange_index_status(workspace);
    let recent_operations = read_log(workspace, 10).unwrap_or_default();
    let data = StatusData {
        root: workspace.root.to_string_lossy().into_owned(),
        git,
        index_status,
        recent_operations,
    };
    let summary = if data.git.is_repo {
        format!(
            "branch {}, {} dirty file(s), {} untracked file(s), index {}",
            data.git.branch.as_deref().unwrap_or("unknown"),
            data.git.dirty_files.len(),
            data.git.untracked_files.len(),
            data.index_status.status
        )
    } else {
        "not a git repository".to_string()
    };
    let observation = Observation {
        kind: "workspace_status".to_string(),
        scope: data.root.clone(),
        summary,
        data,
        evidence: vec![],
        truncated: false,
        next_observations: vec![
            "workspace map".to_string(),
            "workspace diff --summary".to_string(),
            "workspace index status".to_string(),
            "workspace log".to_string(),
        ],
    };

    append_log(
        workspace,
        "observe",
        "status",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_status)
}

fn cmd_search(workspace: &Workspace, args: SearchArgs) -> Result<()> {
    let (matches, truncated) = rg_search(workspace, &args.query, args.max_results)?;
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
        matches,
    };
    let summary = format!(
        "{} match(es) for {:?}",
        data.matches.len() + usize::from(truncated),
        data.query
    );
    let next_observations = data
        .matches
        .iter()
        .take(5)
        .map(|m| format!("workspace read {} --lines {}:{}", m.path, m.line, m.line))
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

    append_log(
        workspace,
        "observe",
        "search",
        &args.query,
        &observation.summary,
        None,
    )?;
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

    append_log(
        workspace,
        "observe",
        "index status",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_index_status)
}

fn cmd_index_cochange(workspace: &Workspace, args: IndexCochangeArgs) -> Result<()> {
    if !workspace.is_git_repo {
        bail!("workspace index cochange requires a git repository");
    }

    let index = build_cochange_index(workspace, args.max_commits, args.max_files_per_commit)?;
    let index_path = workspace.cochange_index_path();
    let index_dir = workspace.root.join(INDEX_DIR);
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("failed to create index directory {}", index_dir.display()))?;
    fs::write(&index_path, serde_json::to_string_pretty(&index)?)
        .with_context(|| format!("failed to write index {}", index_path.display()))?;

    let summary = format!(
        "indexed {} co-change edge(s) from {} commit(s)",
        index.edges.len(),
        index.commits_indexed
    );
    let observation = Observation {
        kind: "workspace_index_cochange".to_string(),
        scope: workspace.relative(&index_path),
        summary,
        data: index,
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
        .take(5)
        .map(|file| format!("workspace read {}", file.path))
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

    append_log(
        workspace,
        "observe",
        "related",
        &target,
        &observation.summary,
        None,
    )?;
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
        .take(5)
        .map(|file| format!("workspace read {}", file.path))
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

    append_log(
        workspace,
        "observe",
        "impact",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_impact)
}

fn cmd_read(workspace: &Workspace, args: ReadArgs) -> Result<()> {
    let path = workspace.resolve_existing_workspace_path(&args.path)?;
    let full_content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read text file {}", path.display()))?;
    let rel_path = workspace.relative(&path);
    let range = args
        .lines
        .as_deref()
        .map(parse_line_range)
        .transpose()
        .context("invalid --lines range")?;
    let (content, line_label, truncated) = if let Some((start, end)) = range {
        let selected = full_content
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                let line_number = idx + 1;
                (line_number >= start && line_number <= end).then_some(line)
            })
            .collect::<Vec<_>>()
            .join("\n");
        (selected, Some(format!("{start}:{end}")), false)
    } else {
        (full_content, None, false)
    };

    let data = ReadData {
        path: rel_path.clone(),
        lines: line_label.clone(),
        content,
    };
    let summary = match &data.lines {
        Some(lines) => format!("read {} lines {}", data.path, lines),
        None => format!("read {}", data.path),
    };
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
        truncated,
        next_observations: vec![
            format!("workspace search {}", shell_hint(&rel_path)),
            "workspace diff --summary".to_string(),
        ],
    };

    append_log(
        workspace,
        "observe",
        "read",
        &rel_path,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_read)
}

fn cmd_diff(workspace: &Workspace, args: DiffArgs) -> Result<()> {
    let data = if workspace.is_git_repo {
        let summary = git_output(workspace, ["diff", "--stat"])?;
        let files = git_output(workspace, ["diff", "--name-only"])?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let patch = if args.summary {
            None
        } else {
            Some(git_output(workspace, ["diff"])?)
        };
        DiffData {
            is_repo: true,
            summary,
            files,
            patch,
        }
    } else {
        DiffData {
            is_repo: false,
            summary: "not a git repository".to_string(),
            files: vec![],
            patch: None,
        }
    };
    let summary = if data.is_repo {
        format!("{} changed file(s)", data.files.len())
    } else {
        data.summary.clone()
    };
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
        .take(5)
        .map(|path| format!("workspace read {}", path))
        .collect();
    let observation = Observation {
        kind: "workspace_diff".to_string(),
        scope: workspace.root.to_string_lossy().into_owned(),
        summary,
        data,
        evidence,
        truncated: false,
        next_observations,
    };

    append_log(
        workspace,
        "observe",
        "diff",
        &observation.scope,
        &observation.summary,
        None,
    )?;
    output_observation(args.json, &observation, print_diff)
}

fn cmd_patch(workspace: &Workspace, args: PatchArgs) -> Result<()> {
    let patch_path = workspace.resolve_path(&args.patch_file);
    let patch_content = fs::read_to_string(&patch_path)
        .with_context(|| format!("failed to read patch {}", patch_path.display()))?;
    let files_changed = extract_patch_files(&patch_content);
    run_git_apply(workspace, &patch_path, ["--check"])?;
    run_git_apply(workspace, &patch_path, [])?;

    let transaction_id = new_id("tx");
    let transaction_dir = workspace.transaction_dir();
    fs::create_dir_all(&transaction_dir).with_context(|| {
        format!(
            "failed to create transaction directory {}",
            transaction_dir.display()
        )
    })?;
    let stored_patch = transaction_dir.join(format!("{transaction_id}.patch"));
    fs::write(&stored_patch, patch_content)
        .with_context(|| format!("failed to store patch {}", stored_patch.display()))?;

    let data = PatchData {
        transaction_id: transaction_id.clone(),
        patch_file: workspace.relative(&patch_path),
        stored_patch: workspace.relative(&stored_patch),
        files_changed: files_changed.clone(),
    };
    let summary = format!(
        "applied patch transaction {} touching {} file(s)",
        data.transaction_id,
        data.files_changed.len()
    );
    let observation = Observation {
        kind: "workspace_patch".to_string(),
        scope: data.patch_file.clone(),
        summary,
        data,
        evidence: files_changed
            .iter()
            .map(|path| Evidence {
                path: path.clone(),
                lines: None,
                reason: "patch file target".to_string(),
            })
            .collect(),
        truncated: false,
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

fn cmd_run(workspace: &Workspace, args: RunArgs) -> Result<()> {
    let start = Instant::now();
    let output = shell_command(&args.command)
        .current_dir(&workspace.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run command {:?}", args.command))?;
    let duration_ms = start.elapsed().as_millis();
    let stdout = truncate_string(
        &String::from_utf8_lossy(&output.stdout),
        MAX_CAPTURED_OUTPUT,
    );
    let stderr = truncate_string(
        &String::from_utf8_lossy(&output.stderr),
        MAX_CAPTURED_OUTPUT,
    );
    let data = RunData {
        command: args.command.clone(),
        cwd: workspace.root.to_string_lossy().into_owned(),
        exit_code: output.status.code(),
        duration_ms,
        stdout,
        stderr,
    };
    let summary = format!(
        "command exited with {} in {}ms",
        data.exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        data.duration_ms
    );
    let observation = Observation {
        kind: "workspace_run".to_string(),
        scope: data.command.clone(),
        summary,
        data,
        evidence: vec![],
        truncated: false,
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
    let entries = read_log(workspace, args.limit)?;
    let data = LogData {
        log_path: workspace.relative(&workspace.log_path()),
        entries,
    };
    let observation = Observation {
        kind: "workspace_log".to_string(),
        scope: data.log_path.clone(),
        summary: format!("{} operation(s)", data.entries.len()),
        data,
        evidence: vec![],
        truncated: false,
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

    let patch_content = fs::read_to_string(&stored_patch)
        .with_context(|| format!("failed to read stored patch {}", stored_patch.display()))?;
    let files_changed = extract_patch_files(&patch_content);
    run_git_apply(workspace, &stored_patch, ["--reverse", "--check"])?;
    run_git_apply(workspace, &stored_patch, ["--reverse"])?;

    let rollback_transaction_id = new_id("rb");
    let data = RollbackData {
        transaction_id: args.transaction_id.clone(),
        rollback_transaction_id: rollback_transaction_id.clone(),
        stored_patch: workspace.relative(&stored_patch),
        files_changed: files_changed.clone(),
    };
    let summary = format!(
        "rolled back transaction {} touching {} file(s)",
        data.transaction_id,
        data.files_changed.len()
    );
    let observation = Observation {
        kind: "workspace_rollback".to_string(),
        scope: data.transaction_id.clone(),
        summary,
        data,
        evidence: files_changed
            .iter()
            .map(|path| Evidence {
                path: path.clone(),
                lines: None,
                reason: "rollback target".to_string(),
            })
            .collect(),
        truncated: false,
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
            large_files.push(LargeFile {
                path: rel.clone(),
                bytes: metadata.len(),
            });
        }
        if let Ok(modified) = metadata.modified() {
            recent_candidates.push((modified, rel.clone()));
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
    let structure = detect_structure(&files, directories.into_iter().collect());
    let commands = detect_commands(workspace, &files)?;
    let important_files = important_files(&structure, &stack);

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
    })
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
        if let Ok(text) = fs::read_to_string(package_json) {
            for (needle, name) in [
                ("\"next\"", "nextjs"),
                ("\"react\"", "react"),
                ("\"vue\"", "vue"),
                ("\"svelte\"", "svelte"),
                ("\"vite\"", "vite"),
                ("\"express\"", "express"),
            ] {
                if text.contains(needle) {
                    frameworks.insert(name.to_string());
                }
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

    let entrypoints = files
        .iter()
        .filter(|path| entrypoint_names.contains(&path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let tests = files
        .iter()
        .filter(|path| is_test_file(path))
        .cloned()
        .collect::<Vec<_>>();
    let configs = files
        .iter()
        .filter(|path| config_names.contains(&path.as_str()) || path.ends_with(".config.js"))
        .cloned()
        .collect::<Vec<_>>();
    let docs = files
        .iter()
        .filter(|path| {
            let lower = path.to_lowercase();
            lower == "readme.md" || lower.starts_with("docs/") || lower.ends_with(".md")
        })
        .cloned()
        .collect::<Vec<_>>();

    StructureSummary {
        directories,
        entrypoints,
        tests,
        configs,
        docs,
    }
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
        if let Ok(text) = fs::read_to_string(package_json)
            && let Ok(value) = serde_json::from_str::<Value>(&text)
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
        next.push("workspace read README.md".to_string());
    }
    for file in map.important_files.iter().take(4) {
        if file.path != "README.md" {
            next.push(format!("workspace read {}", file.path));
        }
    }
    if map.git.is_repo {
        next.push("workspace diff --summary".to_string());
        next.push("workspace index status".to_string());
        next.push("workspace index cochange".to_string());
        next.push("workspace impact --diff --by cochange".to_string());
        if let Some(entrypoint) = map.structure.entrypoints.first() {
            next.push(format!("workspace related {} --by cochange", entrypoint));
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
    rank: RankingMethod,
) -> RelatedData {
    let commits_matched = output
        .related
        .iter()
        .map(|item| item.cochanges)
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
        related: output
            .related
            .into_iter()
            .map(related_file_from_related_cli)
            .collect(),
    }
}

fn related_file_from_related_cli(item: RelatedCliItem) -> RelatedFile {
    RelatedFile {
        path: normalize_repo_path(&item.path),
        score: round3(item.score),
        cochanged_commits: item.cochanges,
        weighted_cochanges: round3(item.weight),
        sample_commits: item
            .evidence
            .iter()
            .map(|evidence| short_commit(&evidence.hash).to_string())
            .collect(),
    }
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
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read co-change index {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse co-change index {}", path.display()))
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
            if seed_set.contains(&path) {
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
    let log = git_output(
        workspace,
        vec![
            "log".to_string(),
            "--format=commit:%H".to_string(),
            "--name-only".to_string(),
            format!("--max-count={}", max_commits.max(1)),
            "--".to_string(),
        ],
    )?;
    Ok(parse_git_log_name_only(&log))
}

fn git_changed_files(workspace: &Workspace) -> Result<Vec<String>> {
    let mut files = BTreeSet::new();
    collect_git_name_only(workspace, ["diff", "--name-only"], &mut files)?;
    collect_git_name_only(workspace, ["diff", "--cached", "--name-only"], &mut files)?;

    let status = git_output(workspace, ["status", "--porcelain"])?;
    for line in status.lines() {
        if line.len() < 4 {
            continue;
        }
        if &line[..2] == "??" {
            let path = normalize_repo_path(&line[3..]);
            if should_include_repo_file(&path) {
                files.insert(path);
            }
        }
    }

    Ok(files.into_iter().collect())
}

fn collect_git_name_only<const N: usize>(
    workspace: &Workspace,
    args: [&str; N],
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for line in git_output(workspace, args)?.lines() {
        let path = normalize_repo_path(line);
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

fn parse_git_log_name_only(output: &str) -> Vec<GitCommitFiles> {
    let mut commits = Vec::new();
    let mut current_hash: Option<String> = None;
    let mut current_files = BTreeSet::new();

    for line in output.lines() {
        if let Some(hash) = line.strip_prefix("commit:") {
            push_commit(&mut commits, &mut current_hash, &mut current_files);
            current_hash = Some(hash.trim().to_string());
            continue;
        }

        let path = normalize_repo_path(line.trim());
        if should_include_repo_file(&path) {
            current_files.insert(path);
        }
    }

    push_commit(&mut commits, &mut current_hash, &mut current_files);
    commits
}

fn push_commit(
    commits: &mut Vec<GitCommitFiles>,
    current_hash: &mut Option<String>,
    current_files: &mut BTreeSet<String>,
) {
    if let Some(hash) = current_hash.take() {
        commits.push(GitCommitFiles {
            hash,
            files: std::mem::take(current_files).into_iter().collect(),
        });
    }
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
        .trim_matches('/')
        .to_string()
}

fn should_include_repo_file(path: &str) -> bool {
    !path.is_empty()
        && path != LOG_DIR
        && !path.starts_with(&format!("{LOG_DIR}/"))
        && !path.starts_with(".git/")
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
            dirty_files: vec![],
            untracked_files: vec![],
        });
    }

    let branch = git_output(workspace, ["branch", "--show-current"])
        .ok()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty());
    let status = git_output(workspace, ["status", "--porcelain"])?;
    let mut dirty_files = Vec::new();
    let mut untracked_files = Vec::new();

    for line in status.lines() {
        if line.len() < 4 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].to_string();
        if path == LOG_DIR || path.starts_with(&format!("{LOG_DIR}/")) {
            continue;
        }
        if code == "??" {
            untracked_files.push(path);
        } else {
            dirty_files.push(path);
        }
    }

    Ok(GitSummary {
        is_repo: true,
        branch,
        dirty_files,
        untracked_files,
    })
}

fn rg_search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
) -> Result<(Vec<SearchMatch>, bool)> {
    let output = Command::new("rg")
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
        .output()
        .context("failed to run ripgrep")?;

    if !output.status.success() && output.status.code() != Some(1) {
        bail!(
            "ripgrep failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut matches = Vec::new();
    let mut truncated = false;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let value: Value = serde_json::from_str(line).context("failed to parse ripgrep JSON")?;
        if value.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let data = value.get("data").unwrap_or(&Value::Null);
        let path = data
            .get("path")
            .and_then(|path| path.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_start_matches("./")
            .to_string();
        let text = data
            .get("lines")
            .and_then(|lines| lines.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_end_matches('\n')
            .to_string();
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

        if matches.len() >= max_results {
            truncated = true;
            break;
        }
        matches.push(SearchMatch {
            path,
            line: line_number,
            column,
            text,
        });
    }

    Ok((matches, truncated))
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

fn extract_patch_files(patch_content: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in patch_content.lines() {
        if let Some(path) = line.strip_prefix("+++ b/").and_then(clean_patch_path) {
            files.insert(path);
        } else if let Some(path) = line.strip_prefix("--- a/").and_then(clean_patch_path) {
            files.insert(path);
        } else if let Some(path) = line.strip_prefix("rename from ").and_then(clean_patch_path) {
            files.insert(path);
        } else if let Some(path) = line.strip_prefix("rename to ").and_then(clean_patch_path) {
            files.insert(path);
        } else if let Some((old_path, new_path)) = diff_git_paths(line) {
            if let Some(path) = clean_patch_path(old_path) {
                files.insert(path);
            }
            if let Some(path) = clean_patch_path(new_path) {
                files.insert(path);
            }
        }
    }
    files.into_iter().collect()
}

fn diff_git_paths(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("diff --git ")?;
    let rest = rest.strip_prefix("a/")?;
    rest.rsplit_once(" b/")
}

fn clean_patch_path(raw: &str) -> Option<String> {
    let path = raw.split_once('\t').map_or(raw, |(path, _)| path);
    if path.is_empty() || path == "/dev/null" {
        None
    } else {
        Some(path.to_string())
    }
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
    let output = command.output().context("failed to run git apply")?;
    if !output.status.success() {
        bail!(
            "git apply failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn git_output<I, S>(workspace: &Workspace, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(&workspace.root)
        .args(args)
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
        scope: scope.to_string(),
        summary: summary.to_string(),
        transaction_id: transaction_id.map(ToOwned::to_owned),
    };
    let line = serde_json::to_string(&entry)?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(workspace.log_path())
        .with_context(|| format!("failed to open {}", workspace.log_path().display()))?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn read_log(workspace: &Workspace, limit: usize) -> Result<Vec<LogEntry>> {
    let path = workspace.log_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read log {}", path.display()))?;
    let mut entries = parse_log_entries(&text)
        .with_context(|| format!("failed to parse operation log {}", path.display()))?;
    if entries.len() > limit {
        entries = entries.split_off(entries.len() - limit);
    }
    Ok(entries)
}

fn parse_log_entries(text: &str) -> Result<Vec<LogEntry>> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(idx, line)| {
            serde_json::from_str::<LogEntry>(line)
                .with_context(|| format!("invalid operation log JSON at line {}", idx + 1))
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
                map.git.dirty_files.len(),
                map.git.untracked_files.len()
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
        print_list("untracked", &data.git.untracked_files);
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

fn print_index_cochange(observation: &Observation<CochangeIndex>) -> Result<()> {
    let data = &observation.data;
    println!("{}", observation.summary);
    println!("  head: {}", data.head.as_deref().unwrap_or("unknown"));
    println!("  scanned: {} commit(s)", data.commits_scanned);
    println!("  indexed: {} commit(s)", data.commits_indexed);
    println!("  ignored broad commits: {}", data.ignored_large_commits);
    println!("  files: {}", data.file_commit_counts.len());
    println!("  edges: {}", data.edges.len());
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
    Ok(())
}

fn print_rollback(observation: &Observation<RollbackData>) -> Result<()> {
    println!("{}", observation.summary);
    println!(
        "  rollback transaction: {}",
        observation.data.rollback_transaction_id
    );
    print_list("files", &observation.data.files_changed);
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

fn shell_hint(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '-' | '_'))
    {
        value.to_string()
    } else {
        format!("{value:?}")
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
