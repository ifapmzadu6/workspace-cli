use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        let output = Command::new(&self.bin)
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
            .output()
            .with_context(|| format!("failed to run related-cli at {}", self.bin.display()))?;

        if !output.status.success() {
            bail!(
                "related-cli query failed for {}: {}",
                target,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        serde_json::from_slice(&output.stdout)
            .with_context(|| "failed to parse related-cli JSON output")
    }
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
