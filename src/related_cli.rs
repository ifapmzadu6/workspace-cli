use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

const MAX_RELATED_CLI_STDOUT: usize = 1_000_000;
const MAX_RELATED_CLI_STDERR: usize = 24_000;

#[derive(Deserialize)]
pub(crate) struct RelatedCliOutput {
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) related: Vec<RelatedCliItem>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RelatedCliItem {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) score: f64,
    #[serde(default)]
    pub(crate) cochanges: usize,
    #[serde(default)]
    pub(crate) weight: f64,
    #[serde(default)]
    pub(crate) evidence: Vec<RelatedCliEvidence>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RelatedCliEvidence {
    pub(crate) hash: String,
}

pub(crate) struct RelatedCli {
    bin: PathBuf,
    history_backend: String,
}

impl RelatedCli {
    pub(crate) fn detect() -> Option<Self> {
        if env_flag_is_set("WORKSPACE_RELATED_DISABLE") {
            return None;
        }
        let bin = if let Ok(value) = env::var("WORKSPACE_RELATED_BIN") {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }
            PathBuf::from(trimmed)
        } else {
            let candidate = PathBuf::from("related");
            if !command_available(&candidate) {
                return None;
            }
            candidate
        };
        let history_backend =
            env::var("WORKSPACE_RELATED_HISTORY_BACKEND").unwrap_or_else(|_| "git".to_string());
        Some(Self {
            bin,
            history_backend,
        })
    }

    pub(crate) fn query(
        &self,
        repo_root: &Path,
        target: &str,
        max_commits: usize,
        max_files_per_commit: usize,
        max_results: usize,
        mode: &str,
    ) -> Result<RelatedCliOutput> {
        let child = Command::new(&self.bin)
            .current_dir(repo_root)
            .arg("query")
            .arg(target)
            .arg("--repo")
            .arg(repo_root)
            .arg("--history-backend")
            .arg(&self.history_backend)
            .arg("--max-commits")
            .arg(max_commits.to_string())
            .arg("--max-files-per-commit")
            .arg(max_files_per_commit.to_string())
            .arg("--top")
            .arg(max_results.to_string())
            .arg("--mode")
            .arg(mode)
            .arg("--evidence")
            .arg("3")
            .arg("--json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run related-cli at {}", self.bin.display()))?;
        let output = wait_for_related_cli_query(child)?;

        if !output.status.success() {
            bail!(
                "related-cli query failed for {}: {}",
                target,
                output.stderr.display_text().trim()
            );
        }
        if output.stdout.truncated {
            bail!(
                "related-cli JSON output exceeded {} bytes",
                MAX_RELATED_CLI_STDOUT
            );
        }

        serde_json::from_slice(&output.stdout.bytes)
            .with_context(|| "failed to parse related-cli JSON output")
    }
}

struct CapturedCommandOutput {
    status: ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

type CapturedOutputReader = std::thread::JoinHandle<Result<CapturedOutput>>;

impl CapturedOutput {
    fn display_text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).into_owned();
        if self.truncated {
            text.push_str("\n[output truncated]\n");
        }
        text
    }
}

fn wait_for_related_cli_query(mut child: Child) -> Result<CapturedCommandOutput> {
    let stdout_reader = capture_related_cli_stdout(&mut child)?;
    let stderr_reader = capture_related_cli_stderr(&mut child)?;
    let status = child
        .wait()
        .context("failed to wait for related-cli query")?;
    let stdout = join_output_reader(stdout_reader, "related-cli stdout")?;
    let stderr = join_output_reader(stderr_reader, "related-cli stderr")?;

    Ok(CapturedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn capture_related_cli_stdout(child: &mut Child) -> Result<CapturedOutputReader> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture related-cli stdout"))?;
    Ok(std::thread::spawn(move || {
        read_output_bytes_with_limit(stdout, MAX_RELATED_CLI_STDOUT)
    }))
}

fn capture_related_cli_stderr(child: &mut Child) -> Result<CapturedOutputReader> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture related-cli stderr"))?;
    Ok(std::thread::spawn(move || {
        read_output_bytes_with_limit(stderr, MAX_RELATED_CLI_STDERR)
    }))
}

fn join_output_reader(reader: CapturedOutputReader, stream_name: &str) -> Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| anyhow!("{stream_name} reader thread panicked"))?
}

fn read_output_bytes_with_limit<R: Read>(
    mut reader: R,
    max_bytes: usize,
) -> Result<CapturedOutput> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut truncated = false;

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .context("failed to read related-cli output")?;
        if bytes_read == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(bytes.len());
        if remaining > 0 {
            let bytes_to_store = remaining.min(bytes_read);
            bytes.extend_from_slice(&buffer[..bytes_to_store]);
        }
        if bytes_read > remaining {
            truncated = true;
        }
    }

    Ok(CapturedOutput { bytes, truncated })
}

fn command_available(bin: &Path) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn env_flag_is_set(name: &str) -> bool {
    env::var(name)
        .map(|value| !value.trim().is_empty() && value != "0" && value != "false")
        .unwrap_or(false)
}
