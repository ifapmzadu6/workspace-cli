use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn workspace_bin() -> &'static str {
    env!("CARGO_BIN_EXE_workspace")
}

fn run_workspace(cwd: &Path, args: &[&str]) -> Value {
    let output = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_DISABLE", "1")
        .output()
        .expect("workspace command should run");

    assert!(
        output.status.success(),
        "workspace {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("workspace output should be JSON")
}

fn run_workspace_failure(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_DISABLE", "1")
        .output()
        .expect("workspace command should run");

    assert!(
        !output.status.success(),
        "workspace {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run_workspace_with_related_bin(cwd: &Path, args: &[&str], related_bin: &Path) -> Value {
    let output = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_BIN", related_bin)
        .output()
        .expect("workspace command should run");

    assert!(
        output.status.success(),
        "workspace {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("workspace output should be JSON")
}

fn run(cwd: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("{program} should run: {error}"));

    assert!(
        output.status.success(),
        "{program} {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_file(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

fn append_file(root: &Path, path: &str, content: &str) {
    use std::io::Write;

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(root.join(path))
        .expect("file should open for append");
    file.write_all(content.as_bytes())
        .expect("file append should succeed");
}

fn init_git_repo() -> TempDir {
    let temp = TempDir::new().expect("temp dir should be created");
    run(temp.path(), "git", &["init", "-q"]);
    run(
        temp.path(),
        "git",
        &["config", "user.email", "test@example.com"],
    );
    run(temp.path(), "git", &["config", "user.name", "Test"]);
    temp
}

fn commit_all(root: &Path, message: &str) {
    run(root, "git", &["add", "."]);
    run(root, "git", &["commit", "-m", message, "-q"]);
}

#[cfg(unix)]
fn write_executable(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, content).expect("executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("permissions should be set");
}

#[test]
fn map_and_read_emit_observations() {
    let temp = TempDir::new().expect("temp dir should be created");
    write_file(temp.path(), "README.md", "# demo\n\nhello\n");
    write_file(
        temp.path(),
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    write_file(temp.path(), "src/main.rs", "fn main() {}\n");

    let map = run_workspace(temp.path(), &["map", "--json"]);
    assert_eq!(map["kind"], "workspace_map");
    assert!(
        strings_at(&map, &["data", "stack", "package_managers"]).contains(&"cargo".to_string()),
        "map should detect cargo package manager: {map}"
    );
    assert!(
        map["next_observations"]
            .as_array()
            .expect("next observations should be an array")
            .iter()
            .any(|item| item == "workspace read README.md")
    );

    let read = run_workspace(
        temp.path(),
        &["read", "README.md", "--lines", "1:1", "--json"],
    );
    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["data"]["content"], "# demo");
}

#[test]
fn search_reports_total_matches_when_results_are_truncated() {
    let temp = TempDir::new().expect("temp dir should be created");
    write_file(temp.path(), "a.txt", "needle one\nneedle two\n");
    write_file(temp.path(), "b.txt", "needle three\n");

    let search = run_workspace(
        temp.path(),
        &["search", "needle", "--max-results", "2", "--json"],
    );
    let matches = search["data"]["matches"]
        .as_array()
        .expect("matches should be an array");

    assert_eq!(search["kind"], "workspace_search");
    assert_eq!(search["data"]["total_matches"], 3);
    assert_eq!(matches.len(), 2);
    assert_eq!(search["truncated"], true);
    assert!(
        search["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("3 match(es)")
    );
    assert!(
        search["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("showing 2")
    );
}

#[test]
fn read_rejects_paths_outside_workspace() {
    let workspace = TempDir::new().expect("workspace temp dir should be created");
    let outside = TempDir::new().expect("outside temp dir should be created");
    write_file(workspace.path(), "inside.txt", "inside\n");
    write_file(outside.path(), "outside.txt", "outside\n");

    let read = run_workspace(workspace.path(), &["read", "inside.txt", "--json"]);
    assert_eq!(read["data"]["content"], "inside\n");

    let stderr = run_workspace_failure(
        workspace.path(),
        &[
            "read",
            outside
                .path()
                .join("outside.txt")
                .to_str()
                .expect("path should be utf-8"),
            "--json",
        ],
    );
    assert!(
        stderr.contains("outside workspace root"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn related_rejects_paths_outside_workspace() {
    let parent = TempDir::new().expect("parent temp dir should be created");
    let root = parent.path().join("workspace");
    fs::create_dir(&root).expect("workspace dir should be created");
    run(&root, "git", &["init", "-q"]);
    run(&root, "git", &["config", "user.email", "test@example.com"]);
    run(&root, "git", &["config", "user.name", "Test"]);
    write_file(&root, "src/a.rs", "a\n");
    commit_all(&root, "initial");
    write_file(parent.path(), "outside.rs", "outside\n");

    let relative_stderr = run_workspace_failure(&root, &["related", "../outside.rs", "--json"]);
    assert!(
        relative_stderr.contains("outside workspace root"),
        "unexpected stderr: {relative_stderr}"
    );

    let absolute_stderr = run_workspace_failure(
        &root,
        &[
            "related",
            parent
                .path()
                .join("outside.rs")
                .to_str()
                .expect("path should be utf-8"),
            "--json",
        ],
    );
    assert!(
        absolute_stderr.contains("outside workspace root"),
        "unexpected stderr: {absolute_stderr}"
    );
}

#[test]
fn index_related_impact_and_status_cover_cochange_flow() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "src/a.rs", "a1\n");
    write_file(root, "src/b.rs", "b1\n");
    commit_all(root, "a with b");

    append_file(root, "src/b.rs", "b2\n");
    write_file(root, "src/c.rs", "c1\n");
    commit_all(root, "b with c");

    append_file(root, "src/b.rs", "b3\n");
    write_file(root, "tests/b_test.rs", "test1\n");
    commit_all(root, "b with test");

    let missing_status = run_workspace(root, &["status", "--json"]);
    assert_eq!(missing_status["data"]["index_status"]["status"], "missing");

    let index = run_workspace(root, &["index", "cochange", "--json"]);
    assert_eq!(index["kind"], "workspace_index_cochange");
    assert_eq!(index["data"]["commits_indexed"], 3);

    let fresh_status = run_workspace(root, &["status", "--json"]);
    assert_eq!(fresh_status["data"]["index_status"]["status"], "fresh");
    assert_eq!(fresh_status["data"]["index_status"]["fresh"], true);

    let related = run_workspace(
        root,
        &[
            "related", "src/a.rs", "--by", "cochange", "--rank", "pagerank", "--json",
        ],
    );
    let related_paths = paths_at(&related, &["data", "related"]);
    assert_eq!(related["data"]["relationship_source"], "cochange-index");
    assert_eq!(related["data"]["ranking"], "pagerank");
    assert!(related_paths.contains(&"src/b.rs".to_string()));
    assert!(related_paths.contains(&"src/c.rs".to_string()));

    append_file(root, "src/a.rs", "local change\n");
    let impact = run_workspace(
        root,
        &[
            "impact", "--diff", "--by", "cochange", "--rank", "pagerank", "--json",
        ],
    );
    let impacted_paths = paths_at(&impact, &["data", "impacted"]);
    assert_eq!(impact["data"]["seed_files"][0], "src/a.rs");
    assert!(impacted_paths.contains(&"src/b.rs".to_string()));
    assert!(impacted_paths.contains(&"tests/b_test.rs".to_string()));
}

#[cfg(unix)]
#[test]
fn related_can_delegate_to_related_cli() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "src/a.rs", "a\n");
    write_file(root, "src/b.rs", "b\n");
    commit_all(root, "a with b");

    let bin_dir = TempDir::new().expect("bin temp dir should be created");
    let fake_related = bin_dir.path().join("fake-related");
    write_executable(
        &fake_related,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "related 0.0.0-test"
  exit 0
fi
cat <<'JSON'
{
  "target": "src/a.rs",
  "mode": "direct:on-demand:GitCli",
  "related": [
    {
      "path": "src/b.rs",
      "score": 0.75,
      "cochanges": 2,
      "weight": 1.5,
      "last_seen": "2026-05-24T00:00:00+09:00",
      "reason": "direct_cochange",
      "evidence": [
        {
          "hash": "1234567890abcdef",
          "date": "2026-05-24T00:00:00+09:00",
          "subject": "a with b",
          "file_count": 2,
          "weight": 1.5
        }
      ]
    }
  ]
}
JSON
"#,
    );

    let related = run_workspace_with_related_bin(
        root,
        &["related", "src/a.rs", "--by", "cochange", "--json"],
        &fake_related,
    );

    assert_eq!(related["kind"], "workspace_related");
    assert!(
        related["data"]["relationship_source"]
            .as_str()
            .expect("relationship source should be a string")
            .starts_with("related-cli:")
    );
    assert_eq!(related["data"]["related"][0]["path"], "src/b.rs");
    assert_eq!(related["data"]["related"][0]["cochanged_commits"], 2);
    assert_eq!(
        related["data"]["related"][0]["sample_commits"][0],
        "1234567890ab"
    );

    append_file(root, "src/a.rs", "local change\n");
    let impact = run_workspace_with_related_bin(
        root,
        &["impact", "--diff", "--by", "cochange", "--json"],
        &fake_related,
    );
    assert_eq!(impact["kind"], "workspace_impact");
    assert!(
        impact["data"]["relationship_source"]
            .as_str()
            .expect("relationship source should be a string")
            .starts_with("related-cli:direct:aggregate")
    );
    assert!(strings_at(&impact, &["data", "seed_files"]).contains(&"src/a.rs".to_string()));
    assert_eq!(impact["data"]["impacted"][0]["path"], "src/b.rs");
}

#[test]
fn patch_run_log_diff_and_rollback_cover_transaction_flow() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "change.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
",
    );

    let patch = run_workspace(
        root,
        &[
            "patch",
            "--description",
            "update note",
            "change.patch",
            "--json",
        ],
    );
    assert_eq!(patch["kind"], "workspace_patch");
    assert_eq!(patch["data"]["files_changed"][0], "note.txt");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello workspace\n"
    );
    let transaction_id = patch["data"]["transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    assert_eq!(diff["kind"], "workspace_diff");
    assert!(strings_at(&diff, &["data", "files"]).contains(&"note.txt".to_string()));
    assert!(diff["data"]["patch"].is_null());

    let run = run_workspace(root, &["run", "printf verified", "--json"]);
    assert_eq!(run["kind"], "workspace_run");
    assert_eq!(run["data"]["exit_code"], 0);
    assert_eq!(run["data"]["stdout"], "verified");

    let log = run_workspace(root, &["log", "--json"]);
    let ops = strings_at(&log, &["data", "entries"])
        .into_iter()
        .collect::<Vec<_>>();
    assert!(ops.iter().any(|entry| entry.contains("patch")));
    assert!(ops.iter().any(|entry| entry.contains("run")));

    let rollback = run_workspace(root, &["rollback", &transaction_id, "--json"]);
    assert_eq!(rollback["kind"], "workspace_rollback");
    assert_eq!(rollback["data"]["transaction_id"], transaction_id);
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );

    let clean_diff = run_workspace(root, &["diff", "--summary", "--json"]);
    assert!(
        clean_diff["data"]["files"]
            .as_array()
            .expect("diff files should be an array")
            .is_empty()
    );
}

#[test]
fn diff_excludes_workspace_metadata_changes() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    write_file(root, ".workspace/log.jsonl", "old\n");
    commit_all(root, "initial note and metadata");
    write_file(root, "note.txt", "hello workspace\n");
    write_file(root, ".workspace/log.jsonl", "new\n");

    let diff = run_workspace(root, &["diff", "--json"]);
    let files = strings_at(&diff, &["data", "files"]);
    let patch = diff["data"]["patch"]
        .as_str()
        .expect("diff patch should be a string");

    assert_eq!(diff["kind"], "workspace_diff");
    assert_eq!(files, vec!["note.txt".to_string()]);
    assert!(patch.contains("diff --git a/note.txt b/note.txt"));
    assert!(!patch.contains(".workspace/log.jsonl"));
}

#[test]
fn patch_does_not_apply_when_transaction_storage_fails() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "change.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
",
    );
    write_file(root, ".workspace/transactions", "not a directory\n");

    let stderr = run_workspace_failure(
        root,
        &[
            "patch",
            "--description",
            "update note",
            "change.patch",
            "--json",
        ],
    );

    assert!(
        stderr.contains("failed to create transaction directory"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn patch_does_not_apply_when_operation_log_is_not_writable() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "change.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
",
    );
    fs::create_dir_all(root.join(".workspace/log.jsonl"))
        .expect("log path directory should be created");

    let stderr = run_workspace_failure(
        root,
        &[
            "patch",
            "--description",
            "update note",
            "change.patch",
            "--json",
        ],
    );

    assert!(
        stderr.contains("failed to open"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn patch_rejects_patch_files_outside_workspace() {
    let temp = init_git_repo();
    let root = temp.path();
    let outside = TempDir::new().expect("outside temp dir should be created");

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        outside.path(),
        "change.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
",
    );

    let stderr = run_workspace_failure(
        root,
        &[
            "patch",
            "--description",
            "update note",
            outside
                .path()
                .join("change.patch")
                .to_str()
                .expect("path should be utf-8"),
            "--json",
        ],
    );

    assert!(
        stderr.contains("outside workspace root"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn patch_rejects_workspace_metadata_targets() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");
    write_file(
        root,
        "metadata.patch",
        "\
diff --git a/.workspace/log.jsonl b/.workspace/log.jsonl
new file mode 100644
--- /dev/null
+++ b/.workspace/log.jsonl
@@ -0,0 +1 @@
+corrupt
",
    );

    let stderr = run_workspace_failure(
        root,
        &[
            "patch",
            "--description",
            "modify metadata",
            "metadata.patch",
            "--json",
        ],
    );

    assert!(
        stderr.contains("outside observable workspace files"),
        "unexpected stderr: {stderr}"
    );
    assert!(!root.join(".workspace/log.jsonl").exists());
}

#[test]
fn patch_reports_files_from_binary_patch_headers() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");

    fs::create_dir_all(root.join("assets")).expect("assets directory should be created");
    fs::write(root.join("assets/logo.bin"), b"\0workspace").expect("binary file should be written");
    run(root, "git", &["add", "assets/logo.bin"]);
    let diff = Command::new("git")
        .current_dir(root)
        .args(["diff", "--cached", "--binary"])
        .output()
        .expect("git diff should run");
    assert!(
        diff.status.success(),
        "git diff failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff.stdout),
        String::from_utf8_lossy(&diff.stderr)
    );
    fs::write(root.join("binary.patch"), diff.stdout).expect("binary patch should be written");
    run(root, "git", &["reset", "-q"]);
    fs::remove_file(root.join("assets/logo.bin")).expect("staged binary file should be removed");

    let patch = run_workspace(
        root,
        &[
            "patch",
            "--description",
            "add binary asset",
            "binary.patch",
            "--json",
        ],
    );

    assert_eq!(patch["kind"], "workspace_patch");
    assert!(
        strings_at(&patch, &["data", "files_changed"]).contains(&"assets/logo.bin".to_string())
    );
    assert!(root.join("assets/logo.bin").exists());
}

#[test]
fn patch_reports_files_from_quoted_git_paths() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");

    let quoted_path = "src/tab\tname.txt";
    write_file(root, quoted_path, "quoted\n");
    run(root, "git", &["add", quoted_path]);
    let diff = Command::new("git")
        .current_dir(root)
        .args(["diff", "--cached"])
        .output()
        .expect("git diff should run");
    assert!(
        diff.status.success(),
        "git diff failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff.stdout),
        String::from_utf8_lossy(&diff.stderr)
    );
    fs::write(root.join("quoted.patch"), diff.stdout).expect("quoted patch should be written");
    run(root, "git", &["reset", "-q"]);
    fs::remove_file(root.join(quoted_path)).expect("staged quoted file should be removed");

    let patch = run_workspace(
        root,
        &[
            "patch",
            "--description",
            "add quoted path",
            "quoted.patch",
            "--json",
        ],
    );

    assert_eq!(patch["kind"], "workspace_patch");
    assert!(strings_at(&patch, &["data", "files_changed"]).contains(&quoted_path.to_string()));
    assert!(root.join(quoted_path).exists());
}

#[test]
fn run_records_nonzero_exit_without_failing_cli() {
    let temp = TempDir::new().expect("temp dir should be created");

    let run = run_workspace(temp.path(), &["run", "printf fail >&2; exit 7", "--json"]);

    assert_eq!(run["kind"], "workspace_run");
    assert_eq!(run["data"]["command"], "printf fail >&2; exit 7");
    assert_eq!(run["data"]["exit_code"], 7);
    assert_eq!(run["data"]["stdout"], "");
    assert_eq!(run["data"]["stderr"], "fail");

    let log = run_workspace(temp.path(), &["log", "--json"]);
    let entries = strings_at(&log, &["data", "entries"]);
    assert!(
        entries
            .iter()
            .any(|entry| entry.contains("command exited with 7")),
        "log should record the child exit status: {entries:?}"
    );
}

#[test]
fn run_does_not_execute_when_operation_log_is_not_writable() {
    let temp = TempDir::new().expect("temp dir should be created");
    fs::create_dir_all(temp.path().join(".workspace/log.jsonl"))
        .expect("log path directory should be created");

    let stderr = run_workspace_failure(temp.path(), &["run", "touch side-effect", "--json"]);

    assert!(
        stderr.contains("failed to open"),
        "unexpected stderr: {stderr}"
    );
    assert!(!temp.path().join("side-effect").exists());
}

#[test]
fn rollback_does_not_apply_when_operation_log_is_not_writable() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "change.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
",
    );

    let patch = run_workspace(
        root,
        &[
            "patch",
            "--description",
            "update note",
            "change.patch",
            "--json",
        ],
    );
    let transaction_id = patch["data"]["transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();
    fs::remove_file(root.join(".workspace/log.jsonl")).expect("log file should be removed");
    fs::create_dir(root.join(".workspace/log.jsonl"))
        .expect("log path directory should be created");

    let stderr = run_workspace_failure(root, &["rollback", &transaction_id, "--json"]);

    assert!(
        stderr.contains("failed to open"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello workspace\n"
    );
}

#[test]
fn rollback_rejects_invalid_transaction_ids() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");

    for transaction_id in [
        "/tmp/not-a-transaction",
        "../tx-123",
        "rb-123",
        "tx-",
        "tx-not-digits",
    ] {
        let stderr = run_workspace_failure(root, &["rollback", transaction_id, "--json"]);
        assert!(
            stderr.contains("invalid transaction id"),
            "unexpected stderr for {transaction_id:?}: {stderr}"
        );
    }
}

#[test]
fn log_parse_errors_include_line_number() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(
        root,
        ".workspace/log.jsonl",
        "{\"id\":\"ok\",\"timestamp_unix_ms\":1,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"ok\",\"transaction_id\":null}\nnot json\n",
    );

    let stderr = run_workspace_failure(root, &["log", "--json"]);
    assert!(
        stderr.contains("failed to parse operation log"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("line 2"),
        "expected line number in stderr: {stderr}"
    );
}

#[test]
fn status_reports_operation_log_parse_errors() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");
    write_file(
        root,
        ".workspace/log.jsonl",
        "{\"id\":\"ok\",\"timestamp_unix_ms\":1,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"ok\",\"transaction_id\":null}\nnot json\n",
    );

    let status = run_workspace(root, &["status", "--json"]);
    let error = status["data"]["recent_operations_error"]
        .as_str()
        .expect("status should expose the log parse error");

    assert_eq!(status["kind"], "workspace_status");
    assert!(
        status["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("operation log unreadable")
    );
    assert!(
        error.contains("failed to parse operation log") && error.contains("line 2"),
        "unexpected recent operations error: {error}"
    );
}

#[test]
fn log_limit_ignores_corrupt_entries_outside_requested_window() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(
        root,
        ".workspace/log.jsonl",
        "\
not json
{\"id\":\"op-1\",\"timestamp_unix_ms\":1,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"one\",\"transaction_id\":null}
{\"id\":\"op-2\",\"timestamp_unix_ms\":2,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"two\",\"transaction_id\":null}
{\"id\":\"op-3\",\"timestamp_unix_ms\":3,\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"three\",\"transaction_id\":null}
",
    );

    let log = run_workspace(root, &["log", "--limit", "2", "--json"]);
    let entries = log["data"]["entries"]
        .as_array()
        .expect("log entries should be an array");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["id"], "op-2");
    assert_eq!(entries[1]["id"], "op-3");
}

fn paths_at(value: &Value, path: &[&str]) -> Vec<String> {
    let mut cursor = value;
    for segment in path {
        cursor = &cursor[*segment];
    }
    cursor
        .as_array()
        .expect("target should be an array")
        .iter()
        .map(|item| {
            item["path"]
                .as_str()
                .expect("path should be a string")
                .to_string()
        })
        .collect()
}

fn strings_at(value: &Value, path: &[&str]) -> Vec<String> {
    let mut cursor = value;
    for segment in path {
        cursor = &cursor[*segment];
    }
    cursor
        .as_array()
        .expect("target should be an array")
        .iter()
        .map(|item| match item.as_str() {
            Some(value) => value.to_string(),
            None => item.to_string(),
        })
        .collect()
}
