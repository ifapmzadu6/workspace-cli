use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
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

fn run_workspace_failure_with_stdin(cwd: &Path, args: &[&str], stdin: &str) -> String {
    let mut child = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_DISABLE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("workspace command should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("stdin should be written");
    let output = child
        .wait_with_output()
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

fn run_workspace_with_stdin(cwd: &Path, args: &[&str], stdin: &str) -> Value {
    let mut child = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_DISABLE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("workspace command should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(stdin.as_bytes())
        .expect("stdin should be written");
    let output = child
        .wait_with_output()
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

fn run_workspace_failure_with_related_bin(cwd: &Path, args: &[&str], related_bin: &Path) -> String {
    let output = Command::new(workspace_bin())
        .current_dir(cwd)
        .args(args)
        .env("WORKSPACE_RELATED_BIN", related_bin)
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

fn write_sized_file(root: &Path, path: &str, len: u64) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    let file = fs::File::create(path).expect("file should be created");
    file.set_len(len).expect("file size should be set");
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
fn map_detects_package_json_stack_and_commands() {
    let temp = TempDir::new().expect("temp dir should be created");
    write_file(
        temp.path(),
        "package.json",
        r#"{
  "scripts": {
    "test": "vitest run",
    "dev": "vite --host 0.0.0.0"
  },
  "dependencies": {
    "react": "^19.0.0",
    "vite": "^6.0.0"
  }
}
"#,
    );
    write_file(temp.path(), "src/index.js", "console.log('demo');\n");

    let map = run_workspace(temp.path(), &["map", "--json"]);
    let package_managers = strings_at(&map, &["data", "stack", "package_managers"]);
    let frameworks = strings_at(&map, &["data", "stack", "frameworks"]);

    assert_eq!(map["kind"], "workspace_map");
    assert!(package_managers.contains(&"npm".to_string()));
    assert!(frameworks.contains(&"react".to_string()));
    assert!(frameworks.contains(&"vite".to_string()));
    assert_eq!(map["data"]["commands"]["test"], "npm run test # vitest run");
    assert_eq!(
        map["data"]["commands"]["dev"],
        "npm run dev # vite --host 0.0.0.0"
    );
}

#[test]
fn map_does_not_suggest_reading_workspace_root() {
    let temp = TempDir::new().expect("temp dir should be created");

    let map = run_workspace(temp.path(), &["map", "--json"]);
    let next = strings_at(&map, &["next_observations"]);
    let important = paths_at(&map, &["data", "important_files"]);

    assert_eq!(map["kind"], "workspace_map");
    assert!(important.contains(&".".to_string()));
    assert!(!next.contains(&"workspace read .".to_string()));
}

#[test]
fn map_truncates_large_observation_lists() {
    let temp = TempDir::new().expect("temp dir should be created");
    let root = temp.path();

    for index in 0..90 {
        write_file(root, &format!("dir_{index:03}/file.txt"), "content\n");
        write_file(root, &format!("docs/page_{index:03}.md"), "doc\n");
        write_file(root, &format!("tests/case_{index:03}.rs"), "test\n");
    }
    for index in 0..45 {
        write_sized_file(root, &format!("large/blob_{index:03}.bin"), 1_000_001);
    }

    let map = run_workspace(root, &["map", "--json"]);

    assert_eq!(map["kind"], "workspace_map");
    assert_eq!(map["truncated"], true);
    assert!(
        map["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("map truncated")
    );
    assert_eq!(
        map["data"]["structure"]["directories"]
            .as_array()
            .expect("directories should be an array")
            .len(),
        80
    );
    assert_eq!(
        map["data"]["structure"]["docs"]
            .as_array()
            .expect("docs should be an array")
            .len(),
        80
    );
    assert_eq!(
        map["data"]["structure"]["tests"]
            .as_array()
            .expect("tests should be an array")
            .len(),
        80
    );
    assert_eq!(
        map["data"]["stats"]["large_files"]
            .as_array()
            .expect("large files should be an array")
            .len(),
        40
    );
    assert_eq!(map["data"]["omitted"]["directories"], 13);
    assert_eq!(map["data"]["omitted"]["docs"], 10);
    assert_eq!(map["data"]["omitted"]["tests"], 10);
    assert_eq!(map["data"]["omitted"]["large_files"], 5);
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
fn search_counts_many_matches_with_limited_results() {
    let temp = TempDir::new().expect("temp dir should be created");
    let content = (0..250)
        .map(|index| format!("needle {index}\n"))
        .collect::<String>();
    write_file(temp.path(), "many.txt", &content);

    let search = run_workspace(
        temp.path(),
        &["search", "needle", "--max-results", "3", "--json"],
    );
    let matches = search["data"]["matches"]
        .as_array()
        .expect("matches should be an array");

    assert_eq!(search["kind"], "workspace_search");
    assert_eq!(search["data"]["total_matches"], 250);
    assert_eq!(matches.len(), 3);
    assert_eq!(search["truncated"], true);
}

#[test]
fn search_quotes_read_suggestions_for_paths_that_need_shell_quoting() {
    let temp = TempDir::new().expect("temp dir should be created");
    write_file(temp.path(), "space name.txt", "needle\n");

    let search = run_workspace(temp.path(), &["search", "needle", "--json"]);
    let next = strings_at(&search, &["next_observations"]);

    assert_eq!(search["kind"], "workspace_search");
    assert_eq!(search["data"]["matches"][0]["path"], "space name.txt");
    assert!(next.contains(&"workspace read 'space name.txt' --lines 1:1".to_string()));
}

#[test]
fn search_truncates_large_match_text() {
    let temp = TempDir::new().expect("temp dir should be created");
    let line = format!("needle {} tail\n", "a".repeat(3_000));
    write_file(temp.path(), "large.txt", &line);

    let search = run_workspace(temp.path(), &["search", "needle", "--json"]);
    let text = search["data"]["matches"][0]["text"]
        .as_str()
        .expect("match text should be a string");

    assert_eq!(search["kind"], "workspace_search");
    assert_eq!(search["truncated"], true);
    assert_eq!(search["data"]["truncated_match_texts"], 1);
    assert!(
        search["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("truncated 1 match text")
    );
    assert!(text.contains("[output truncated]"));
    assert!(!text.contains("tail"));
}

#[test]
fn search_handles_oversized_match_lines() {
    let temp = TempDir::new().expect("temp dir should be created");
    let line = format!("needle {} tail\n", "a".repeat(80_000));
    write_file(temp.path(), "large.txt", &line);

    let search = run_workspace(temp.path(), &["search", "needle", "--json"]);
    let text = search["data"]["matches"][0]["text"]
        .as_str()
        .expect("match text should be a string");

    assert_eq!(search["kind"], "workspace_search");
    assert_eq!(search["data"]["total_matches"], 1);
    assert_eq!(search["truncated"], true);
    assert_eq!(search["data"]["truncated_match_texts"], 1);
    assert!(text.contains("[output truncated]"));
    assert!(!text.contains("tail"));
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
fn read_truncates_large_content() {
    let temp = TempDir::new().expect("temp dir should be created");
    let content = format!("{}tail\n", "a".repeat(30_000));
    write_file(temp.path(), "large.txt", &content);

    let read = run_workspace(temp.path(), &["read", "large.txt", "--json"]);
    let returned = read["data"]["content"]
        .as_str()
        .expect("read content should be a string");

    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["truncated"], true);
    assert!(
        read["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("truncated")
    );
    assert!(returned.len() < content.len());
    assert!(returned.contains("[output truncated]"));
    assert!(!returned.contains("tail"));
}

#[test]
fn read_truncates_large_line_range() {
    let temp = TempDir::new().expect("temp dir should be created");
    let content = format!("first\n{}tail\nlast\n", "b".repeat(30_000));
    write_file(temp.path(), "large.txt", &content);

    let read = run_workspace(
        temp.path(),
        &["read", "large.txt", "--lines", "2:2", "--json"],
    );
    let returned = read["data"]["content"]
        .as_str()
        .expect("read content should be a string");

    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["data"]["lines"], "2:2");
    assert_eq!(read["truncated"], true);
    assert!(returned.contains("[output truncated]"));
    assert!(!returned.contains("tail"));
    assert!(!returned.contains("last"));
}

#[test]
fn read_line_range_skips_large_unselected_lines() {
    let temp = TempDir::new().expect("temp dir should be created");
    let content = format!("{}\ntarget\n", "a".repeat(1_000_000));
    write_file(temp.path(), "large.txt", &content);

    let read = run_workspace(
        temp.path(),
        &["read", "large.txt", "--lines", "2:2", "--json"],
    );

    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["truncated"], false);
    assert_eq!(read["data"]["content"], "target");
}

#[test]
fn read_line_range_handles_crlf_across_read_buffer() {
    let temp = TempDir::new().expect("temp dir should be created");
    let first_line = "x".repeat(8191);
    let content = format!("{first_line}\r\nsecond\r\n");
    write_file(temp.path(), "crlf.txt", &content);

    let read = run_workspace(
        temp.path(),
        &["read", "crlf.txt", "--lines", "1:1", "--json"],
    );

    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["truncated"], false);
    assert_eq!(read["data"]["content"], first_line);
}

#[test]
fn read_succeeds_when_operation_log_is_not_writable() {
    let temp = TempDir::new().expect("temp dir should be created");
    write_file(temp.path(), "note.txt", "hello\n");
    fs::create_dir_all(temp.path().join(".workspace/log.jsonl"))
        .expect("log path directory should be created");

    let read = run_workspace(temp.path(), &["read", "note.txt", "--json"]);

    assert_eq!(read["kind"], "workspace_read");
    assert_eq!(read["data"]["content"], "hello\n");
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

    let ensured_related = run_workspace(
        root,
        &[
            "related",
            "src/a.rs",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--ensure-index",
            "--max-commits",
            "1000",
            "--max-results",
            "1",
            "--include-content",
            "--max-content-files",
            "1",
            "--json",
        ],
    );
    assert_eq!(
        ensured_related["data"]["relationship_source"],
        "cochange-index"
    );
    assert_eq!(ensured_related["data"]["ranking"], "hybrid");
    assert_eq!(
        ensured_related["data"]["included_content"][0]["path"],
        "src/b.rs"
    );
    assert!(
        ensured_related["data"]["included_content"][0]["content"]
            .as_str()
            .expect("included content should be a string")
            .contains("b3")
    );
    let ensured_status = run_workspace(root, &["status", "--json"]);
    assert_eq!(ensured_status["data"]["index_status"]["status"], "fresh");
    assert_eq!(ensured_status["data"]["index_status"]["fresh"], true);

    let index = run_workspace(root, &["index", "cochange", "--json"]);
    assert_eq!(index["kind"], "workspace_index_cochange");
    assert_eq!(index["data"]["commits_indexed"], 3);
    assert_eq!(index["data"]["file_count"], 4);
    assert_eq!(index["data"]["edge_count"], 3);
    assert!(index["data"].get("edges").is_none());
    assert!(index["data"].get("file_commit_counts").is_none());
    let saved_index = fs::read_to_string(root.join(".workspace/index/cochange.json"))
        .expect("saved co-change index should be readable");
    let saved_index: Value =
        serde_json::from_str(&saved_index).expect("saved co-change index should be JSON");
    assert_eq!(
        saved_index["edges"]
            .as_array()
            .expect("saved index should retain edges")
            .len(),
        3
    );

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

    let related_with_content = run_workspace(
        root,
        &[
            "related",
            "src/a.rs",
            "--by",
            "cochange",
            "--use-index",
            "--rank",
            "hybrid",
            "--max-results",
            "1",
            "--include-content",
            "--max-content-files",
            "1",
            "--json",
        ],
    );
    let included_content = related_with_content["data"]["included_content"]
        .as_array()
        .expect("included_content should be an array");
    assert_eq!(included_content.len(), 1);
    assert_eq!(included_content[0]["path"], "src/b.rs");
    assert!(
        included_content[0]["content"]
            .as_str()
            .expect("included content should be a string")
            .contains("b3")
    );
    assert_eq!(included_content[0]["truncated"], false);
    assert!(
        related_with_content["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("included content for 1 file(s)")
    );
    assert!(
        strings_at(&related_with_content, &["next_observations"])
            .iter()
            .all(|item| item != "workspace read src/b.rs")
    );

    let hybrid_related = run_workspace(
        root,
        &[
            "related", "src/a.rs", "--by", "cochange", "--rank", "hybrid", "--json",
        ],
    );
    let hybrid_related_paths = paths_at(&hybrid_related, &["data", "related"]);
    assert_eq!(
        hybrid_related["data"]["relationship_source"],
        "cochange-index"
    );
    assert_eq!(hybrid_related["data"]["ranking"], "hybrid");
    assert!(hybrid_related_paths.contains(&"src/b.rs".to_string()));
    assert!(hybrid_related_paths.contains(&"src/c.rs".to_string()));

    let weighted_hybrid_related = run_workspace(
        root,
        &[
            "related",
            "src/a.rs",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--hybrid-direct-weight",
            "1.0",
            "--json",
        ],
    );
    assert_eq!(weighted_hybrid_related["data"]["ranking"], "hybrid");
    assert!(
        paths_at(&weighted_hybrid_related, &["data", "related"]).contains(&"src/b.rs".to_string())
    );

    let invalid_weight_stderr = run_workspace_failure(
        root,
        &[
            "related",
            "src/a.rs",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--hybrid-direct-weight",
            "1.1",
            "--json",
        ],
    );
    assert!(
        invalid_weight_stderr.contains("--hybrid-direct-weight must be between 0.0 and 1.0"),
        "unexpected stderr: {invalid_weight_stderr}"
    );

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

    let hybrid_impact = run_workspace(
        root,
        &[
            "impact", "--diff", "--by", "cochange", "--rank", "hybrid", "--json",
        ],
    );
    let hybrid_impacted_paths = paths_at(&hybrid_impact, &["data", "impacted"]);
    assert_eq!(
        hybrid_impact["data"]["relationship_source"],
        "cochange-index"
    );
    assert_eq!(hybrid_impact["data"]["ranking"], "hybrid");
    assert!(hybrid_impacted_paths.contains(&"src/b.rs".to_string()));
    assert!(hybrid_impacted_paths.contains(&"tests/b_test.rs".to_string()));
}

#[test]
fn impact_decodes_git_quoted_seed_paths() {
    let temp = init_git_repo();
    let root = temp.path();
    let seed = "src/tab\tname.rs";

    write_file(root, seed, "seed\n");
    write_file(root, "src/neighbor.rs", "neighbor\n");
    commit_all(root, "seed with neighbor");
    append_file(root, seed, "changed\n");

    let impact = run_workspace(root, &["impact", "--diff", "--by", "cochange", "--json"]);
    let seed_files = strings_at(&impact, &["data", "seed_files"]);
    let impacted_paths = paths_at(&impact, &["data", "impacted"]);

    assert_eq!(impact["kind"], "workspace_impact");
    assert!(
        seed_files.contains(&seed.to_string()),
        "seed files should decode git quoting: {seed_files:?}"
    );
    assert!(impacted_paths.contains(&"src/neighbor.rs".to_string()));
}

#[test]
fn impact_expands_untracked_directories_to_files() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "README.md", "initial\n");
    commit_all(root, "initial");
    write_file(root, "new/nested/file.rs", "new\n");

    let impact = run_workspace(root, &["impact", "--diff", "--by", "cochange", "--json"]);
    let seed_files = strings_at(&impact, &["data", "seed_files"]);

    assert_eq!(impact["kind"], "workspace_impact");
    assert!(seed_files.contains(&"new/nested/file.rs".to_string()));
    assert!(!seed_files.contains(&"new".to_string()));
    assert!(!seed_files.contains(&"new/nested".to_string()));
}

#[test]
fn impact_bounds_large_seed_file_lists() {
    let temp = init_git_repo();
    let root = temp.path();

    for index in 0..90 {
        write_file(root, &format!("src/file_{index:03}.rs"), "initial\n");
    }
    commit_all(root, "initial");
    for index in 0..90 {
        append_file(root, &format!("src/file_{index:03}.rs"), "changed\n");
    }

    let impact = run_workspace(root, &["impact", "--diff", "--by", "cochange", "--json"]);
    let seed_files = strings_at(&impact, &["data", "seed_files"]);

    assert_eq!(impact["kind"], "workspace_impact");
    assert_eq!(impact["truncated"], true);
    assert_eq!(impact["data"]["seed_file_count"], 90);
    assert_eq!(impact["data"]["omitted_seed_files"], 10);
    assert_eq!(seed_files.len(), 80);
    assert!(seed_files.contains(&"src/file_000.rs".to_string()));
    assert!(!seed_files.contains(&"src/file_089.rs".to_string()));
    assert!(
        impact["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("90 seed file(s)")
    );
    assert!(
        impact["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("seed files truncated")
    );
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
      "path": ".workspace/log.jsonl",
      "score": 0.99,
      "cochanges": 4,
      "weight": 2.0,
      "evidence": [{"hash": "9999999999999999"}]
    },
    {
      "path": "../outside.rs",
      "score": 0.95,
      "cochanges": 3,
      "weight": 1.9,
      "evidence": [{"hash": "eeeeeeeeeeeeeeee"}]
    },
    {
      "path": "C:\\outside.rs",
      "score": 0.94,
      "cochanges": 3,
      "weight": 1.8,
      "evidence": [{"hash": "dddddddddddddddd"}]
    },
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
        },
        {"hash": "2234567890abcdef"},
        {"hash": "3234567890abcdef"},
        {"hash": "4234567890abcdef"},
        {"hash": "5234567890abcdef"},
        {"hash": "6234567890abcdef"}
      ]
    },
    {
      "path": "src/c.rs",
      "score": 0.50,
      "cochanges": 1,
      "weight": 1.0,
      "evidence": [{"hash": "cccccccccccccccc"}]
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
    let related_paths = paths_at(&related, &["data", "related"]);
    assert!(
        related["data"]["relationship_source"]
            .as_str()
            .expect("relationship source should be a string")
            .starts_with("related-cli:")
    );
    assert!(!related_paths.contains(&".workspace/log.jsonl".to_string()));
    assert!(!related_paths.contains(&"../outside.rs".to_string()));
    assert!(!related_paths.contains(&"C:/outside.rs".to_string()));
    assert!(related_paths.contains(&"src/b.rs".to_string()));
    assert_eq!(related["data"]["related"][0]["path"], "src/b.rs");
    assert_eq!(related["data"]["related"][0]["cochanged_commits"], 2);
    assert_eq!(
        related["data"]["related"][0]["sample_commits"][0],
        "1234567890ab"
    );
    assert_eq!(
        related["data"]["related"][0]["sample_commits"]
            .as_array()
            .expect("sample commits should be an array")
            .len(),
        5
    );

    let limited_related = run_workspace_with_related_bin(
        root,
        &[
            "related",
            "src/a.rs",
            "--by",
            "cochange",
            "--max-results",
            "1",
            "--json",
        ],
        &fake_related,
    );
    let limited_related_paths = paths_at(&limited_related, &["data", "related"]);
    assert_eq!(limited_related_paths, vec!["src/b.rs".to_string()]);

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
    let impacted_paths = paths_at(&impact, &["data", "impacted"]);
    assert!(!impacted_paths.contains(&".workspace/log.jsonl".to_string()));
    assert!(!impacted_paths.contains(&"../outside.rs".to_string()));
    assert!(!impacted_paths.contains(&"C:/outside.rs".to_string()));
    assert!(impacted_paths.contains(&"src/b.rs".to_string()));
    assert_eq!(impact["data"]["impacted"][0]["path"], "src/b.rs");

    let limited_impact = run_workspace_with_related_bin(
        root,
        &[
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--max-results",
            "1",
            "--json",
        ],
        &fake_related,
    );
    let limited_impacted_paths = paths_at(&limited_impact, &["data", "impacted"]);
    assert_eq!(limited_impacted_paths, vec!["src/b.rs".to_string()]);
}

#[cfg(unix)]
#[test]
fn related_cli_json_output_is_bounded() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "src/a.rs", "a\n");
    commit_all(root, "a");

    let bin_dir = TempDir::new().expect("bin temp dir should be created");
    let fake_related = bin_dir.path().join("fake-related");
    write_executable(
        &fake_related,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "related 0.0.0-test"
  exit 0
fi
python3 - <<'PY'
import sys
sys.stdout.write('{"mode":"direct:on-demand:GitCli","related":[')
sys.stdout.write(' ' * 1100000)
sys.stdout.write(']}')
PY
"#,
    );

    let stderr = run_workspace_failure_with_related_bin(
        root,
        &["related", "src/a.rs", "--by", "cochange", "--json"],
        &fake_related,
    );

    assert!(
        stderr.contains("related-cli JSON output exceeded"),
        "stderr should report bounded related-cli output, got:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn related_and_impact_skip_missing_files_in_read_suggestions() {
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
      "path": "src/missing.rs",
      "score": 0.90,
      "cochanges": 3,
      "weight": 1.8,
      "evidence": [{"hash": "aaaaaaaaaaaaaaaa"}]
    },
    {
      "path": "src/b.rs",
      "score": 0.75,
      "cochanges": 2,
      "weight": 1.5,
      "evidence": [{"hash": "bbbbbbbbbbbbbbbb"}]
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
    let related_paths = paths_at(&related, &["data", "related"]);
    let related_next = strings_at(&related, &["next_observations"]);
    assert!(related_paths.contains(&"src/missing.rs".to_string()));
    assert!(related_paths.contains(&"src/b.rs".to_string()));
    assert!(!related_next.contains(&"workspace read src/missing.rs".to_string()));
    assert!(related_next.contains(&"workspace read src/b.rs".to_string()));

    append_file(root, "src/a.rs", "local change\n");
    let impact = run_workspace_with_related_bin(
        root,
        &["impact", "--diff", "--by", "cochange", "--json"],
        &fake_related,
    );
    let impacted_paths = paths_at(&impact, &["data", "impacted"]);
    let impact_next = strings_at(&impact, &["next_observations"]);
    assert!(impacted_paths.contains(&"src/missing.rs".to_string()));
    assert!(impacted_paths.contains(&"src/b.rs".to_string()));
    assert!(!impact_next.contains(&"workspace read src/missing.rs".to_string()));
    assert!(impact_next.contains(&"workspace read src/b.rs".to_string()));
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
fn patch_can_apply_unified_diff_from_stdin() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    let patch_content = "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello stdin
";

    let patch = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch_content);

    assert_eq!(patch["kind"], "workspace_patch");
    assert_eq!(patch["scope"], "<stdin>");
    assert_eq!(patch["data"]["patch_file"], "<stdin>");
    assert_eq!(patch["data"]["files_changed"][0], "note.txt");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello stdin\n"
    );
    let transaction_id = patch["data"]["transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();
    let stored_patch = patch["data"]["stored_patch"]
        .as_str()
        .expect("stored patch should be a string");
    assert_eq!(
        fs::read_to_string(root.join(stored_patch)).expect("stored patch should be readable"),
        patch_content
    );

    let rollback = run_workspace(root, &["rollback", &transaction_id, "--json"]);
    assert_eq!(rollback["kind"], "workspace_rollback");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn trial_rolls_back_failed_verifier_and_includes_patch() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "bad.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+bad
",
    );

    let trial = run_workspace(
        root,
        &[
            "trial",
            "--description",
            "try bad patch",
            "--run",
            "grep -q '^good$' note.txt",
            "--rollback-on-fail",
            "--include-patch",
            "--json",
            "bad.patch",
        ],
    );

    assert_eq!(trial["kind"], "workspace_trial");
    assert_eq!(trial["scope"], "bad.patch");
    assert_eq!(trial["data"]["patch_file"], "bad.patch");
    assert_eq!(trial["data"]["exit_code"], 1);
    assert_eq!(trial["data"]["rolled_back"], true);
    assert!(trial["data"]["rollback_transaction_id"].is_string());
    assert_eq!(trial["data"]["file_count"], 1);
    assert_eq!(trial["data"]["files_changed"][0], "note.txt");
    assert!(
        trial["data"]["patch"]
            .as_str()
            .expect("trial should include patch content")
            .contains("+bad")
    );
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
    assert!(
        strings_at(&trial, &["next_observations"])
            .contains(&"workspace diff --summary".to_string())
    );

    let log = run_workspace(root, &["log", "--limit", "1", "--json"]);
    assert_eq!(log["data"]["entries"][0]["op"], "trial");
    assert_eq!(log["data"]["entries"][0]["summary"], "try bad patch");
}

#[test]
fn trial_keeps_successful_patch_and_can_be_rolled_back() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(
        root,
        "good.patch",
        "\
diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+good
",
    );

    let trial = run_workspace(
        root,
        &[
            "trial",
            "--run",
            "grep -q '^good$' note.txt",
            "--rollback-on-fail",
            "--json",
            "good.patch",
        ],
    );

    assert_eq!(trial["kind"], "workspace_trial");
    assert_eq!(trial["data"]["exit_code"], 0);
    assert_eq!(trial["data"]["rolled_back"], false);
    assert!(trial["data"]["rollback_transaction_id"].is_null());
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "good\n");
    let transaction_id = trial["data"]["patch_transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();
    assert!(
        strings_at(&trial, &["next_observations"])
            .contains(&format!("workspace rollback {transaction_id}"))
    );

    let rollback = run_workspace(root, &["rollback", &transaction_id, "--json"]);
    assert_eq!(rollback["kind"], "workspace_rollback");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
}

#[test]
fn replace_can_apply_exact_replacements_from_stdin_and_rollback() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    write_file(
        root,
        "config.json",
        "{\n  \"enabled\": false,\n  \"label\": \"old\"\n}\n",
    );
    commit_all(root, "initial files");
    let replacements = r#"{
  "replacements": [
    {
      "path": "note.txt",
      "find": "hello",
      "replace": "hello workspace"
    },
    {
      "path": "config.json",
      "find": "\"enabled\": false",
      "replace": "\"enabled\": true"
    },
    {
      "path": "config.json",
      "find": "\"label\": \"old\"",
      "replace": "\"label\": \"new\""
    }
  ]
}
"#;

    let replace = run_workspace_with_stdin(root, &["replace", "--stdin", "--json"], replacements);

    assert_eq!(replace["kind"], "workspace_replace");
    assert_eq!(replace["scope"], "<stdin>");
    assert_eq!(replace["data"]["replace_file"], "<stdin>");
    assert_eq!(replace["data"]["replacement_count"], 3);
    assert_eq!(replace["data"]["file_count"], 2);
    assert!(strings_at(&replace, &["data", "files_changed"]).contains(&"config.json".to_string()));
    assert!(strings_at(&replace, &["data", "files_changed"]).contains(&"note.txt".to_string()));
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello workspace\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("config.json")).unwrap(),
        "{\n  \"enabled\": true,\n  \"label\": \"new\"\n}\n"
    );
    let transaction_id = replace["data"]["transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();
    let stored_patch = replace["data"]["stored_patch"]
        .as_str()
        .expect("stored patch should be a string");
    let stored_patch_content =
        fs::read_to_string(root.join(stored_patch)).expect("stored patch should be readable");
    assert!(stored_patch_content.contains("diff --git a/config.json b/config.json"));
    assert!(stored_patch_content.contains("+hello workspace"));

    let log = run_workspace(root, &["log", "--limit", "1", "--json"]);
    assert_eq!(log["data"]["entries"][0]["op"], "replace");

    let rollback = run_workspace(root, &["rollback", &transaction_id, "--json"]);
    assert_eq!(rollback["kind"], "workspace_rollback");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "hello\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("config.json")).unwrap(),
        "{\n  \"enabled\": false,\n  \"label\": \"old\"\n}\n"
    );
}

#[test]
fn replace_preserves_added_trailing_blank_line_and_rollback_bytes() {
    assert_replace_and_rollback_bytes("hello\n", "hello", "good\n", "good\n\n");
}

#[test]
fn replace_preserves_line_endings_and_rollback_bytes() {
    for (before, find, replacement, after) in [
        ("hello\n\n", "hello", "good", "good\n\n"),
        ("hello\n\n\n", "hello", "good\n", "good\n\n\n\n"),
        ("hello\n\n\n", "\n\n", "\n", "hello\n\n"),
        ("hello\n\n", "\n\n", "\n", "hello\n"),
        ("hello\n\n", "hello\n\n", "", ""),
        ("\n\n", "\n\n", "", ""),
        ("\n\n", "\n\n", "\n", "\n"),
        ("hello\r\n\r\n", "hello", "good", "good\r\n\r\n"),
        ("hello\r\n", "hello", "good\r\n", "good\r\n\r\n"),
    ] {
        assert_replace_and_rollback_bytes(before, find, replacement, after);
    }
}

fn assert_replace_and_rollback_bytes(before: &str, find: &str, replacement: &str, after: &str) {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "note.txt", before);
    commit_all(root, "initial note");
    let input = serde_json::json!([{
        "path": "note.txt",
        "find": find,
        "replace": replacement,
    }])
    .to_string();

    let replace = run_workspace_with_stdin(root, &["replace", "--stdin", "--json"], &input);
    assert_eq!(fs::read(root.join("note.txt")).unwrap(), after.as_bytes());
    assert_eq!(
        replace["data"]["files_changed"],
        serde_json::json!(["note.txt"])
    );

    let transaction_id = replace["data"]["transaction_id"].as_str().unwrap();
    run_workspace(root, &["rollback", transaction_id, "--json"]);
    assert_eq!(fs::read(root.join("note.txt")).unwrap(), before.as_bytes());
}

#[test]
fn replace_rejects_empty_matches_and_missing_final_newlines_without_mutation() {
    for (before, find, replacement, error) in [
        ("", "", "good\n", "find string for note.txt is empty"),
        ("", "hello", "good", "expected exactly one match, found 0"),
        (
            "hello",
            "hello",
            "good\n",
            "before content for note.txt has no final newline",
        ),
        (
            "hello\n",
            "hello\n",
            "good",
            "after content for note.txt has no final newline",
        ),
    ] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "note.txt", before);
        commit_all(root, "initial note");
        let input = serde_json::json!([{
            "path": "note.txt",
            "find": find,
            "replace": replacement,
        }])
        .to_string();

        let stderr =
            run_workspace_failure_with_stdin(root, &["replace", "--stdin", "--json"], &input);
        assert!(stderr.contains(error), "unexpected stderr: {stderr}");
        assert_eq!(fs::read(root.join("note.txt")).unwrap(), before.as_bytes());
        assert!(!root.join(".workspace/transactions").exists());
        assert!(!root.join(".workspace/log.jsonl").exists());
    }
}

#[test]
fn replace_requires_unique_match_unless_occurrence_is_given() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "flags.txt", "flag=false\nflag=false\n");
    commit_all(root, "initial flags");
    let ambiguous = r#"[
  {
    "path": "flags.txt",
    "find": "flag=false",
    "replace": "flag=true"
  }
]
"#;

    let stderr =
        run_workspace_failure_with_stdin(root, &["replace", "--stdin", "--json"], ambiguous);
    assert!(
        stderr.contains("expected exactly one match"),
        "stderr should explain ambiguous replacement: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(root.join("flags.txt")).unwrap(),
        "flag=false\nflag=false\n"
    );

    let second_only = r#"[
  {
    "path": "flags.txt",
    "find": "flag=false",
    "replace": "flag=true",
    "occurrence": 2
  }
]
"#;
    let replace = run_workspace_with_stdin(root, &["replace", "--stdin", "--json"], second_only);
    assert_eq!(replace["kind"], "workspace_replace");
    assert_eq!(
        fs::read_to_string(root.join("flags.txt")).unwrap(),
        "flag=false\nflag=true\n"
    );
}

#[test]
fn patch_and_rollback_truncate_large_file_lists() {
    let temp = init_git_repo();
    let root = temp.path();

    for index in 0..90 {
        write_file(root, &format!("many/file_{index:03}.txt"), "old\n");
    }
    commit_all(root, "initial many files");

    let mut patch_content = String::new();
    for index in 0..90 {
        patch_content.push_str(&format!(
            "\
diff --git a/many/file_{index:03}.txt b/many/file_{index:03}.txt
--- a/many/file_{index:03}.txt
+++ b/many/file_{index:03}.txt
@@ -1 +1 @@
-old
+new
"
        ));
    }
    write_file(root, "many.patch", &patch_content);

    let patch = run_workspace(root, &["patch", "many.patch", "--json"]);
    let transaction_id = patch["data"]["transaction_id"]
        .as_str()
        .expect("transaction id should be a string")
        .to_string();

    assert_eq!(patch["kind"], "workspace_patch");
    assert_eq!(patch["truncated"], true);
    assert_eq!(patch["data"]["file_count"], 90);
    assert_eq!(patch["data"]["omitted_files"], 10);
    assert_eq!(
        patch["data"]["files_changed"]
            .as_array()
            .expect("files changed should be an array")
            .len(),
        80
    );
    assert!(
        patch["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("files truncated")
    );

    let rollback = run_workspace(root, &["rollback", &transaction_id, "--json"]);
    assert_eq!(rollback["kind"], "workspace_rollback");
    assert_eq!(rollback["truncated"], true);
    assert_eq!(rollback["data"]["file_count"], 90);
    assert_eq!(rollback["data"]["omitted_files"], 10);
    assert_eq!(
        rollback["data"]["files_changed"]
            .as_array()
            .expect("rollback files changed should be an array")
            .len(),
        80
    );
    assert!(
        rollback["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("files truncated")
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
fn diff_includes_staged_changes() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "note.txt", "hello\n");
    commit_all(root, "initial note");
    write_file(root, "note.txt", "hello staged\n");
    run(root, "git", &["add", "note.txt"]);

    let diff = run_workspace(root, &["diff", "--json"]);
    let files = strings_at(&diff, &["data", "files"]);
    let patch = diff["data"]["patch"]
        .as_str()
        .expect("diff patch should be a string");

    assert_eq!(diff["kind"], "workspace_diff");
    assert_eq!(files, vec!["note.txt".to_string()]);
    assert!(patch.contains("-hello"));
    assert!(patch.contains("+hello staged"));
}

#[test]
fn diff_truncates_large_patch() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "large.txt", "old\n");
    commit_all(root, "initial large file");
    write_file(root, "large.txt", &format!("{}tail\n", "a".repeat(60_000)));

    let diff = run_workspace(root, &["diff", "--json"]);
    let patch = diff["data"]["patch"]
        .as_str()
        .expect("diff patch should be a string");

    assert_eq!(diff["kind"], "workspace_diff");
    assert_eq!(diff["truncated"], true);
    assert!(
        diff["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("patch truncated")
    );
    assert!(strings_at(&diff, &["data", "files"]).contains(&"large.txt".to_string()));
    assert!(patch.contains("[output truncated]"));
    assert!(!patch.contains("tail"));
}

#[test]
fn diff_truncates_large_summary_stat() {
    let temp = init_git_repo();
    let root = temp.path();

    for index in 0..300 {
        let path = format!("many/files/file_{index:03}_with_a_long_observable_name.txt");
        write_file(root, &path, "old\n");
    }
    commit_all(root, "initial many files");
    for index in 0..300 {
        let path = format!("many/files/file_{index:03}_with_a_long_observable_name.txt");
        write_file(root, &path, "new\n");
    }

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    let stat = diff["data"]["summary"]
        .as_str()
        .expect("diff stat should be a string");

    assert_eq!(diff["kind"], "workspace_diff");
    assert_eq!(diff["truncated"], true);
    assert!(
        diff["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("summary truncated")
    );
    assert!(stat.contains("[output truncated]"));
    assert!(diff["data"]["patch"].is_null());
}

#[test]
fn diff_truncates_large_file_lists() {
    let temp = init_git_repo();
    let root = temp.path();

    for index in 0..90 {
        write_file(root, &format!("many/file_{index:03}.txt"), "old\n");
    }
    commit_all(root, "initial many files");
    for index in 0..90 {
        write_file(root, &format!("many/file_{index:03}.txt"), "new\n");
    }

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    let files = diff["data"]["files"]
        .as_array()
        .expect("diff files should be an array");

    assert_eq!(diff["kind"], "workspace_diff");
    assert_eq!(diff["truncated"], true);
    assert_eq!(diff["data"]["file_count"], 90);
    assert_eq!(diff["data"]["omitted_files"], 10);
    assert_eq!(files.len(), 80);
    assert!(
        diff["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("files truncated")
    );
}

#[test]
fn diff_does_not_suggest_reading_deleted_files() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "deleted.txt", "gone\n");
    write_file(root, "kept.txt", "old\n");
    commit_all(root, "initial files");
    fs::remove_file(root.join("deleted.txt")).expect("file should be removed");
    write_file(root, "kept.txt", "new\n");

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    let next = strings_at(&diff, &["next_observations"]);

    assert_eq!(diff["kind"], "workspace_diff");
    assert!(strings_at(&diff, &["data", "files"]).contains(&"deleted.txt".to_string()));
    assert!(strings_at(&diff, &["data", "files"]).contains(&"kept.txt".to_string()));
    assert!(!next.contains(&"workspace read deleted.txt".to_string()));
    assert!(next.contains(&"workspace read kept.txt".to_string()));
}

#[test]
fn diff_quotes_read_suggestions_for_paths_that_need_shell_quoting() {
    let temp = init_git_repo();
    let root = temp.path();

    write_file(root, "space name.txt", "old\n");
    commit_all(root, "initial file with space");
    write_file(root, "space name.txt", "new\n");

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    let next = strings_at(&diff, &["next_observations"]);

    assert_eq!(diff["kind"], "workspace_diff");
    assert!(strings_at(&diff, &["data", "files"]).contains(&"space name.txt".to_string()));
    assert!(next.contains(&"workspace read 'space name.txt'".to_string()));
}

#[test]
fn diff_decodes_git_quoted_name_only_paths() {
    let temp = init_git_repo();
    let root = temp.path();
    let path = "src/tab\tname.txt";

    write_file(root, path, "old\n");
    commit_all(root, "initial tab path");
    write_file(root, path, "new\n");

    let diff = run_workspace(root, &["diff", "--summary", "--json"]);
    let files = strings_at(&diff, &["data", "files"]);
    let next = strings_at(&diff, &["next_observations"]);

    assert_eq!(diff["kind"], "workspace_diff");
    assert!(
        files.contains(&path.to_string()),
        "files should decode git quoting: {files:?}"
    );
    assert!(next.contains(&format!("workspace read '{path}'")));
}

#[test]
fn status_decodes_git_quoted_paths() {
    let temp = init_git_repo();
    let root = temp.path();
    let path = "src/tab\tname.txt";

    write_file(root, path, "old\n");
    commit_all(root, "initial tab path");
    write_file(root, path, "new\n");

    let status = run_workspace(root, &["status", "--json"]);
    let dirty = strings_at(&status, &["data", "git", "dirty_files"]);

    assert_eq!(status["kind"], "workspace_status");
    assert!(
        dirty.contains(&path.to_string()),
        "dirty files should decode git quoting: {dirty:?}"
    );
}

#[test]
fn status_truncates_large_git_file_lists() {
    let temp = init_git_repo();
    let root = temp.path();

    for index in 0..90 {
        write_file(root, &format!("tracked/file_{index:03}.txt"), "old\n");
    }
    commit_all(root, "initial tracked files");
    for index in 0..90 {
        write_file(root, &format!("tracked/file_{index:03}.txt"), "new\n");
        write_file(root, &format!("untracked/file_{index:03}.txt"), "new\n");
    }

    let status = run_workspace(root, &["status", "--json"]);
    let dirty = strings_at(&status, &["data", "git", "dirty_files"]);
    let untracked = strings_at(&status, &["data", "git", "untracked_files"]);

    assert_eq!(status["kind"], "workspace_status");
    assert_eq!(status["truncated"], true);
    assert!(
        status["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("90 dirty file(s), 90 untracked file(s)")
    );
    assert!(
        status["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("status truncated")
    );
    assert_eq!(status["data"]["git"]["dirty_file_count"], 90);
    assert_eq!(status["data"]["git"]["untracked_file_count"], 90);
    assert_eq!(status["data"]["git"]["omitted_dirty_files"], 10);
    assert_eq!(status["data"]["git"]["omitted_untracked_files"], 10);
    assert_eq!(dirty.len(), 80);
    assert_eq!(untracked.len(), 80);

    let map = run_workspace(root, &["map", "--json"]);
    assert_eq!(map["truncated"], true);
    assert_eq!(map["data"]["git"]["dirty_file_count"], 90);
    assert_eq!(map["data"]["git"]["untracked_file_count"], 90);
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
fn patch_custom_prefixes_report_actual_paths_and_rollback_bytes() {
    for mnemonic in [false, true] {
        let temp = init_git_repo();
        let root = temp.path();
        let paths = [
            "src/note.txt",
            "space name.txt",
            "src/tab\tname.txt",
            "src/quote\"name.txt",
            " leading.txt",
            "trailing.txt ",
            "dir/has b/file.txt",
        ];
        for path in paths {
            write_file(root, path, "old\n");
        }
        commit_all(root, "initial paths");
        for path in paths {
            write_file(root, path, "new\n");
        }
        let mut command = Command::new("git");
        command.current_dir(root);
        if mnemonic {
            command.args(["-c", "diff.mnemonicPrefix=true", "diff"]);
        } else {
            command.args(["diff", "--src-prefix=old/", "--dst-prefix=new/"]);
        }
        let diff = command.output().expect("git diff should run");
        assert!(diff.status.success());
        let patch = String::from_utf8(diff.stdout).unwrap();
        run(root, "git", &["reset", "--hard", "-q"]);
        let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
        let mut reported = strings_at(&applied, &["data", "files_changed"]);
        reported.sort();
        let mut expected = paths.map(str::to_string).to_vec();
        expected.sort();
        assert_eq!(reported, expected);
        for path in paths {
            assert_eq!(fs::read(root.join(path)).unwrap(), b"new\n");
        }
        let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
        run_workspace(root, &["rollback", transaction_id, "--json"]);
        for path in paths {
            assert_eq!(fs::read(root.join(path)).unwrap(), b"old\n");
        }
    }
}

#[test]
fn patch_custom_prefix_metadata_is_rejected_before_transaction_storage() {
    for (old_prefix, new_prefix) in [("old", "new"), ("i", "w")] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "note.txt", "safe\n");
        commit_all(root, "initial note");
        write_file(root, ".workspace/metadata.txt", "old\n");
        // Even the unfixed CLI cannot apply the protected patch: storage is blocked.
        write_file(root, ".workspace/transactions", "storage guard\n");
        let patch = format!(
            "diff --git {old_prefix}/.workspace/metadata.txt {new_prefix}/.workspace/metadata.txt\n--- {old_prefix}/.workspace/metadata.txt\n+++ {new_prefix}/.workspace/metadata.txt\n@@ -1 +1 @@\n-old\n+new\n"
        );
        let mut check = Command::new("git")
            .current_dir(root)
            .args(["apply", "-p1", "--check", "--numstat", "-z", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        check
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .unwrap();
        let check = check.wait_with_output().unwrap();
        assert!(
            check.status.success(),
            "git check failed: {:?}",
            check.stderr
        );
        assert_eq!(check.stdout, b"1\t1\t.workspace/metadata.txt\0");
        let stderr =
            run_workspace_failure_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
        assert!(
            stderr.contains("outside observable workspace files"),
            "unexpected stderr: {stderr}"
        );
        assert_eq!(
            fs::read(root.join(".workspace/metadata.txt")).unwrap(),
            b"old\n"
        );
        assert_eq!(
            fs::read(root.join(".workspace/transactions")).unwrap(),
            b"storage guard\n"
        );
        assert!(!root.join(".workspace/log.jsonl").exists());
    }
}

#[test]
#[cfg(unix)]
fn patch_custom_prefix_binary_mode_and_rename_rollback() {
    use std::os::unix::fs::PermissionsExt;

    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "a/old name.txt", "rename this note\n");
    write_file(root, "mode file.txt", "mode only\n");
    fs::write(root.join("binary.bin"), b"\0before\xff").unwrap();
    commit_all(root, "initial files");
    run(root, "git", &["config", "core.filemode", "true"]);
    fs::create_dir(root.join("b")).unwrap();
    fs::rename(root.join("a/old name.txt"), root.join("b/new name.txt")).unwrap();
    fs::write(root.join("binary.bin"), b"\0after\xfe").unwrap();
    let original_mode = fs::metadata(root.join("mode file.txt"))
        .unwrap()
        .permissions()
        .mode();
    fs::set_permissions(
        root.join("mode file.txt"),
        fs::Permissions::from_mode(original_mode | 0o111),
    )
    .unwrap();
    run(root, "git", &["add", "."]);
    let diff = Command::new("git")
        .current_dir(root)
        .args([
            "diff",
            "--cached",
            "--binary",
            "--find-renames",
            "--src-prefix=old/",
            "--dst-prefix=new/",
        ])
        .output()
        .unwrap();
    assert!(diff.status.success());
    let patch = String::from_utf8(diff.stdout).unwrap();
    assert!(patch.contains("GIT binary patch"));
    assert!(patch.contains("old mode 100644\nnew mode 100755"));
    assert!(patch.contains("rename from a/old name.txt"));
    run(root, "git", &["reset", "--hard", "-q"]);

    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
    assert_eq!(
        strings_at(&applied, &["data", "files_changed"]),
        vec![
            "a/old name.txt",
            "b/new name.txt",
            "binary.bin",
            "mode file.txt"
        ]
    );
    assert!(!root.join("a/old name.txt").exists());
    assert_eq!(
        fs::read(root.join("b/new name.txt")).unwrap(),
        b"rename this note\n"
    );
    assert_eq!(fs::read(root.join("binary.bin")).unwrap(), b"\0after\xfe");
    assert_ne!(
        fs::metadata(root.join("mode file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );

    let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
    run_workspace(root, &["rollback", transaction_id, "--json"]);
    assert_eq!(
        fs::read(root.join("a/old name.txt")).unwrap(),
        b"rename this note\n"
    );
    assert!(!root.join("b/new name.txt").exists());
    assert_eq!(fs::read(root.join("binary.bin")).unwrap(), b"\0before\xff");
    assert_eq!(
        fs::metadata(root.join("mode file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[test]
fn patch_custom_prefix_copy_keeps_repository_relative_paths() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "a/source.txt", "copy this note\n");
    commit_all(root, "initial source");
    let patch = "diff --git old/a/source.txt new/b/copy.txt\nsimilarity index 100%\ncopy from a/source.txt\ncopy to b/copy.txt\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(
        strings_at(&applied, &["data", "files_changed"]),
        vec!["a/source.txt", "b/copy.txt"]
    );
    assert_eq!(
        fs::read(root.join("a/source.txt")).unwrap(),
        b"copy this note\n"
    );
    assert_eq!(
        fs::read(root.join("b/copy.txt")).unwrap(),
        b"copy this note\n"
    );
}

#[test]
fn patch_hunk_body_is_not_a_metadata_header() {
    for metadata in [".workspace", ".git"] {
        let temp = init_git_repo();
        let root = temp.path();
        let before = format!("-- nested/{metadata}/config\n");
        write_file(root, "note.txt", &before);
        commit_all(root, "initial note");
        let patch = format!(
            "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-{before}+ordinary\n"
        );
        let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
        assert_eq!(
            applied["data"]["files_changed"],
            serde_json::json!(["note.txt"])
        );
        assert_eq!(fs::read(root.join("note.txt")).unwrap(), b"ordinary\n");
        let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
        run_workspace(root, &["rollback", transaction_id, "--json"]);
        assert_eq!(fs::read(root.join("note.txt")).unwrap(), before.as_bytes());
        assert_replace_and_rollback_bytes(&before, &before, "ordinary\n", "ordinary\n");
    }
}

#[test]
fn rollback_legacy_transaction_ignores_metadata_like_hunk_body() {
    for metadata in [".workspace", ".git"] {
        let temp = init_git_repo();
        let root = temp.path();
        let before = format!("-- nested/{metadata}/config\n");
        write_file(root, "note.txt", &before);
        commit_all(root, "initial note");
        let patch = format!(
            "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-{before}+ordinary\n"
        );
        // Reproduce the original transaction format without using the new CLI to apply.
        write_file(root, ".workspace/transactions/tx-123.patch", &patch);
        run(
            root,
            "git",
            &["apply", "-p1", ".workspace/transactions/tx-123.patch"],
        );
        let rollback = run_workspace(root, &["rollback", "tx-123", "--json"]);
        assert_eq!(
            rollback["data"]["files_changed"],
            serde_json::json!(["note.txt"])
        );
        assert_eq!(fs::read(root.join("note.txt")).unwrap(), before.as_bytes());
        assert_eq!(
            fs::read(root.join(".workspace/transactions/tx-123.patch")).unwrap(),
            patch.as_bytes()
        );
    }
}

#[test]
fn copy_patch_rollback_removes_only_the_unchanged_copy() {
    for content in [
        b"copy this note\n".as_slice(),
        b"hello\r\n\r\n",
        b"\0binary\xff",
        b"",
    ] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "a/source.txt", "");
        fs::write(root.join("a/source.txt"), content).unwrap();
        commit_all(root, "initial source");
        let patch = "diff --git old/a/source.txt new/b/copy.txt\nsimilarity index 100%\ncopy from a/source.txt\ncopy to b/copy.txt\n";
        let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
        let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
        let stored_patch = root.join(applied["data"]["stored_patch"].as_str().unwrap());
        let original_patch = fs::read(&stored_patch).unwrap();
        assert_eq!(fs::read(root.join("b/copy.txt")).unwrap(), content);

        run_workspace(root, &["rollback", transaction_id, "--json"]);
        assert_eq!(fs::read(root.join("a/source.txt")).unwrap(), content);
        assert!(!root.join("b/copy.txt").exists());
        assert_eq!(fs::read(stored_patch).unwrap(), original_patch);
    }
}

#[test]
fn copy_patch_rollback_conflicts_preserve_files_and_transaction() {
    for change in ["source", "destination", "both", "missing_destination"] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "a/source.txt", "original\n");
        commit_all(root, "initial source");
        let patch = "diff --git a/a/source.txt b/b/copy.txt\nsimilarity index 100%\ncopy from a/source.txt\ncopy to b/copy.txt\n";
        let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
        let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
        let stored_patch = root.join(applied["data"]["stored_patch"].as_str().unwrap());
        let stored_before = fs::read(&stored_patch).unwrap();
        let log_before = fs::read(root.join(".workspace/log.jsonl")).unwrap();
        if matches!(change, "source" | "both") {
            write_file(root, "a/source.txt", "user edit\n");
        }
        if matches!(change, "destination" | "both") {
            write_file(root, "b/copy.txt", "user edit\n");
        }
        if change == "missing_destination" {
            fs::remove_file(root.join("b/copy.txt")).unwrap();
        }
        let source_before = fs::read(root.join("a/source.txt")).unwrap();
        let destination_before = fs::read(root.join("b/copy.txt")).ok();
        let stderr = run_workspace_failure(root, &["rollback", transaction_id, "--json"]);
        assert!(
            stderr.contains("rollback conflict"),
            "unexpected stderr: {stderr}"
        );
        assert_eq!(fs::read(root.join("a/source.txt")).unwrap(), source_before);
        assert_eq!(fs::read(root.join("b/copy.txt")).ok(), destination_before);
        assert_eq!(fs::read(stored_patch).unwrap(), stored_before);
        assert_eq!(
            fs::read(root.join(".workspace/log.jsonl")).unwrap(),
            log_before
        );
    }
}

#[test]
fn copy_rollback_does_not_require_a_git_repository() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "source.txt", "original\n");
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"original\n");
    run_workspace(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original\n");
    assert!(!root.join("copy.txt").exists());
    assert!(!root.join(".git").exists());
}

#[test]
fn copy_rollback_binary_content_in_sha256_repository() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run(root, "git", &["init", "-q", "--object-format=sha256"]);
    run(root, "git", &["config", "user.email", "test@example.com"]);
    run(root, "git", &["config", "user.name", "Test"]);
    fs::write(root.join("source.bin"), b"\0binary\xff").unwrap();
    commit_all(root, "initial source");
    let patch = "diff --git a/source.bin b/copy.bin\nsimilarity index 100%\ncopy from source.bin\ncopy to copy.bin\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(fs::read(root.join("copy.bin")).unwrap(), b"\0binary\xff");
    run_workspace(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(root.join("source.bin")).unwrap(), b"\0binary\xff");
    assert!(!root.join("copy.bin").exists());
}

#[test]
#[cfg(unix)]
fn copy_rollback_preserves_symlink_source_and_target() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "note.txt", "untouched\n");
    std::os::unix::fs::symlink("note.txt", root.join("source.link")).unwrap();
    commit_all(root, "initial symlink");
    let patch = "diff --git a/source.link b/copy.link\nsimilarity index 100%\ncopy from source.link\ncopy to copy.link\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(
        fs::read_link(root.join("copy.link")).unwrap(),
        Path::new("note.txt")
    );
    run_workspace(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        fs::read_link(root.join("source.link")).unwrap(),
        Path::new("note.txt")
    );
    assert!(!root.join("copy.link").exists());
    assert_eq!(fs::read(root.join("note.txt")).unwrap(), b"untouched\n");
}

#[test]
fn copy_rollback_log_failure_keeps_all_transaction_files() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "original\n");
    commit_all(root, "initial source");
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    let stored = root.join(applied["data"]["stored_patch"].as_str().unwrap());
    let files = [
        stored.clone(),
        stored.with_extension("rollback.patch"),
        stored.with_extension("rollback.json"),
    ];
    let before = files
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    fs::remove_file(root.join(".workspace/log.jsonl")).unwrap();
    fs::create_dir(root.join(".workspace/log.jsonl")).unwrap();
    let stderr = run_workspace_failure(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        stderr.contains("failed to open"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original\n");
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"original\n");
    assert_eq!(
        files
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn copy_patch_storage_failure_does_not_create_the_destination() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "original\n");
    commit_all(root, "initial source");
    write_file(root, ".workspace/transactions", "storage guard\n");
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
    let stderr = run_workspace_failure_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert!(
        stderr.contains("failed to create transaction directory"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original\n");
    assert!(!root.join("copy.txt").exists());
    assert_eq!(
        fs::read(root.join(".workspace/transactions")).unwrap(),
        b"storage guard\n"
    );
}

#[test]
fn copy_rollback_preserves_bytes_with_whitespace_fix_configuration() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "original  \n");
    write_file(root, ".gitattributes", "copy.txt -whitespace\n");
    commit_all(root, "initial source");
    run(root, "git", &["config", "apply.whitespace", "fix"]);
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 50%\ncopy from source.txt\ncopy to copy.txt\n--- a/source.txt\n+++ b/copy.txt\n@@ -1 +1 @@\n-original  \n+copy edit  \ndiff --git a/source.txt b/source.txt\n--- a/source.txt\n+++ b/source.txt\n@@ -1 +1 @@\n-original  \n+source edit  \n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"source edit\n");
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"copy edit  \n");
    run_workspace(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original  \n");
    assert!(!root.join("copy.txt").exists());
    assert_eq!(
        fs::read(root.join(".gitattributes")).unwrap(),
        b"copy.txt -whitespace\n"
    );
}

#[test]
fn edited_copy_patch_and_source_edit_rollback_together() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source name.txt", "one\ntwo\nthree\n");
    commit_all(root, "initial source");
    let patch = "diff --git old/source name.txt new/copy name.txt\nsimilarity index 66%\ncopy from source name.txt\ncopy to copy name.txt\n--- old/source name.txt\n+++ new/copy name.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+copied edit\n three\ndiff --git old/source name.txt new/source name.txt\n--- old/source name.txt\n+++ new/source name.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+source edit\n three\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    assert_eq!(
        fs::read(root.join("source name.txt")).unwrap(),
        b"one\nsource edit\nthree\n"
    );
    assert_eq!(
        fs::read(root.join("copy name.txt")).unwrap(),
        b"one\ncopied edit\nthree\n"
    );
    run_workspace(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        fs::read(root.join("source name.txt")).unwrap(),
        b"one\ntwo\nthree\n"
    );
    assert!(!root.join("copy name.txt").exists());
}

#[test]
#[cfg(unix)]
fn copy_transaction_binary_mode_rename_and_conflict_rollback() {
    use std::os::unix::fs::PermissionsExt;

    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "unique copy source\n");
    write_file(root, "old.txt", "unique rename source\n");
    write_file(root, "mode.txt", "mode only\n");
    fs::write(root.join("binary.bin"), b"\0before\xff").unwrap();
    commit_all(root, "initial files");
    fs::copy(root.join("source.txt"), root.join("copy.txt")).unwrap();
    fs::rename(root.join("old.txt"), root.join("new.txt")).unwrap();
    fs::write(root.join("binary.bin"), b"\0after\xfe").unwrap();
    fs::set_permissions(root.join("mode.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    run(root, "git", &["add", "."]);
    let diff = Command::new("git")
        .current_dir(root)
        .args([
            "diff",
            "--cached",
            "--binary",
            "--find-renames",
            "--find-copies",
            "--find-copies-harder",
            "--src-prefix=old/",
            "--dst-prefix=new/",
        ])
        .output()
        .unwrap();
    assert!(diff.status.success());
    let patch = String::from_utf8(diff.stdout).unwrap();
    assert!(patch.contains("copy from source.txt"));
    assert!(patch.contains("rename from old.txt"));
    assert!(patch.contains("GIT binary patch"));
    run(root, "git", &["reset", "--hard", "-q"]);
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
    let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
    write_file(root, "copy.txt", "user change\n");
    let stderr = run_workspace_failure(root, &["rollback", transaction_id, "--json"]);
    assert!(stderr.contains("rollback conflict"));
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"user change\n");
    assert_eq!(fs::read(root.join("binary.bin")).unwrap(), b"\0after\xfe");
    assert!(!root.join("old.txt").exists());
    assert_eq!(
        fs::read(root.join("new.txt")).unwrap(),
        b"unique rename source\n"
    );
    assert_ne!(
        fs::metadata(root.join("mode.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );

    write_file(root, "copy.txt", "unique copy source\n");
    run_workspace(root, &["rollback", transaction_id, "--json"]);
    assert_eq!(
        fs::read(root.join("source.txt")).unwrap(),
        b"unique copy source\n"
    );
    assert_eq!(
        fs::read(root.join("old.txt")).unwrap(),
        b"unique rename source\n"
    );
    assert!(!root.join("copy.txt").exists());
    assert!(!root.join("new.txt").exists());
    assert_eq!(fs::read(root.join("binary.bin")).unwrap(), b"\0before\xff");
    assert_eq!(
        fs::metadata(root.join("mode.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[test]
fn copy_rollback_snapshot_failures_preserve_the_transaction() {
    for change in [
        "guard_missing",
        "guard_invalid",
        "undo_missing",
        "undo_changed",
        "original_changed",
    ] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "source.txt", "original\n");
        commit_all(root, "initial source");
        let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
        let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
        let transaction_id = applied["data"]["transaction_id"].as_str().unwrap();
        let stored = root.join(applied["data"]["stored_patch"].as_str().unwrap());
        let log_before = fs::read(root.join(".workspace/log.jsonl")).unwrap();
        match change {
            "guard_missing" => fs::remove_file(stored.with_extension("rollback.json")).unwrap(),
            "guard_invalid" => {
                fs::write(stored.with_extension("rollback.json"), b"invalid").unwrap()
            }
            "undo_missing" => fs::remove_file(stored.with_extension("rollback.patch")).unwrap(),
            "undo_changed" => {
                fs::write(stored.with_extension("rollback.patch"), b"invalid").unwrap()
            }
            "original_changed" => fs::write(&stored, b"diff --git a/copy.txt b/copy.txt\nnew file mode 100644\n--- /dev/null\n+++ b/copy.txt\n@@ -0,0 +1 @@\n+original\n").unwrap(),
            _ => unreachable!(),
        }
        let original = fs::read(&stored).unwrap();
        let stderr = run_workspace_failure(root, &["rollback", transaction_id, "--json"]);
        assert!(stderr.contains("snapshot"), "unexpected stderr: {stderr}");
        assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original\n");
        assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"original\n");
        assert_eq!(fs::read(stored).unwrap(), original);
        assert_eq!(
            fs::read(root.join(".workspace/log.jsonl")).unwrap(),
            log_before
        );
    }
}

#[test]
fn legacy_copy_rollback_without_snapshot_refuses_to_guess() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "original\n");
    write_file(root, "copy.txt", "original\n");
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
    write_file(root, ".workspace/transactions/tx-123.patch", patch);
    let stderr = run_workspace_failure(root, &["rollback", "tx-123", "--json"]);
    assert!(stderr.contains("rollback snapshot is missing"));
    assert_eq!(fs::read(root.join("source.txt")).unwrap(), b"original\n");
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"original\n");
    assert_eq!(
        fs::read(root.join(".workspace/transactions/tx-123.patch")).unwrap(),
        patch.as_bytes()
    );
    assert!(!root.join(".workspace/log.jsonl").exists());
}

#[test]
#[cfg(unix)]
fn copy_rollback_mode_conflict_preserves_the_destination() {
    use std::os::unix::fs::PermissionsExt;
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "source.txt", "original\n");
    commit_all(root, "initial source");
    let patch = "diff --git a/source.txt b/copy.txt\nsimilarity index 100%\ncopy from source.txt\ncopy to copy.txt\n";
    let applied = run_workspace_with_stdin(root, &["patch", "--stdin", "--json"], patch);
    fs::set_permissions(root.join("copy.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    let stderr = run_workspace_failure(
        root,
        &[
            "rollback",
            applied["data"]["transaction_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert!(stderr.contains("rollback conflict"));
    assert_eq!(fs::read(root.join("copy.txt")).unwrap(), b"original\n");
    assert_ne!(
        fs::metadata(root.join("copy.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[test]
fn patch_custom_prefix_unsafe_paths_are_rejected_without_mutation() {
    for path in [
        ".git/workspace-test",
        ".git",
        "/tmp/workspace-outside-test",
        "../workspace-outside-test",
        "src/../outside.txt",
        "src//note.txt",
    ] {
        let temp = init_git_repo();
        let root = temp.path();
        write_file(root, "note.txt", "safe\n");
        commit_all(root, "initial note");
        let config_before = fs::read(root.join(".git/config")).unwrap();
        write_file(root, ".workspace/transactions", "storage guard\n");
        let patch = format!(
            "diff --git old/{path} new/{path}\nnew file mode 100644\n--- /dev/null\n+++ new/{path}\n@@ -0,0 +1 @@\n+unsafe\n"
        );
        let mut check = Command::new("git")
            .current_dir(root)
            .args(["apply", "-p1", "--check", "--numstat", "-z", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        check
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .unwrap();
        let check = check.wait_with_output().unwrap();
        if check.status.success() {
            assert!(path.contains("//"), "unexpected git acceptance: {path}");
        }
        let stderr =
            run_workspace_failure_with_stdin(root, &["patch", "--stdin", "--json"], &patch);
        assert!(
            stderr.contains("outside observable workspace files"),
            "target {path}, unexpected stderr: {stderr}"
        );
        assert_eq!(fs::read(root.join("note.txt")).unwrap(), b"safe\n");
        assert_eq!(fs::read(root.join(".git/config")).unwrap(), config_before);
        assert_eq!(
            fs::read(root.join(".workspace/transactions")).unwrap(),
            b"storage guard\n"
        );
        assert!(!root.join(".workspace/log.jsonl").exists());
    }
}

#[test]
fn rollback_rejects_custom_prefix_metadata_targets() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "note.txt", "safe\n");
    commit_all(root, "initial note");
    write_file(root, ".workspace/metadata.txt", "new\n");
    write_file(
        root,
        ".workspace/transactions/tx-123.patch",
        "diff --git old/.workspace/metadata.txt new/.workspace/metadata.txt\n--- old/.workspace/metadata.txt\n+++ new/.workspace/metadata.txt\n@@ -1 +1 @@\n-old\n+new\n",
    );
    // A missed validation cannot apply the reverse patch because logging is blocked.
    fs::create_dir(root.join(".workspace/log.jsonl")).unwrap();
    let stderr = run_workspace_failure(root, &["rollback", "tx-123", "--json"]);
    assert!(
        stderr.contains("outside observable workspace files"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read(root.join(".workspace/metadata.txt")).unwrap(),
        b"new\n"
    );
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
fn run_marks_large_output_as_truncated() {
    let temp = TempDir::new().expect("temp dir should be created");

    let run = run_workspace(
        temp.path(),
        &[
            "run",
            "python3 -c \"import sys; sys.stdout.write('a' * 300000); sys.stderr.write('b' * 300000)\"",
            "--json",
        ],
    );
    let stdout = run["data"]["stdout"]
        .as_str()
        .expect("stdout should be a string");
    let stderr = run["data"]["stderr"]
        .as_str()
        .expect("stderr should be a string");

    assert_eq!(run["kind"], "workspace_run");
    assert_eq!(run["truncated"], true);
    assert!(
        run["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("output truncated")
    );
    assert!(stdout.len() < 30_000);
    assert!(stderr.len() < 30_000);
    assert!(stdout.contains("[output truncated]"));
    assert!(stderr.contains("[output truncated]"));
}

#[test]
fn operation_log_truncates_large_scope_and_summary() {
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

    let long_description = format!("{}tail", "d".repeat(3_000));
    let patch = run_workspace(
        root,
        &[
            "patch",
            "--description",
            long_description.as_str(),
            "change.patch",
            "--json",
        ],
    );
    assert_eq!(patch["kind"], "workspace_patch");

    let patch_log = run_workspace(root, &["log", "--limit", "1", "--json"]);
    let patch_entries = patch_log["data"]["entries"]
        .as_array()
        .expect("patch log entries should be an array");
    let patch_summary = patch_entries[0]["summary"]
        .as_str()
        .expect("patch log summary should be a string");
    assert_eq!(patch_entries[0]["op"], "patch");
    assert!(patch_summary.contains("[truncated]"));
    assert!(!patch_summary.contains("tail"));
    assert!(patch_summary.chars().count() < 2_100);

    let long_command = format!("printf ok # {}", "x".repeat(3_000));
    let run = run_workspace(root, &["run", long_command.as_str(), "--json"]);
    assert_eq!(run["kind"], "workspace_run");

    let run_log = run_workspace(root, &["log", "--limit", "1", "--json"]);
    let run_entries = run_log["data"]["entries"]
        .as_array()
        .expect("run log entries should be an array");
    let run_scope = run_entries[0]["scope"]
        .as_str()
        .expect("run log scope should be a string");
    assert_eq!(run_entries[0]["op"], "run");
    assert!(run_scope.contains("[truncated]"));
    assert!(run_scope.chars().count() < 2_100);
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
fn status_succeeds_when_operation_log_is_not_writable() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");
    fs::create_dir_all(root.join(".workspace/log.jsonl"))
        .expect("log path directory should be created");

    let status = run_workspace(root, &["status", "--json"]);
    let error = status["data"]["recent_operations_error"]
        .as_str()
        .expect("status should expose the log read error");

    assert_eq!(status["kind"], "workspace_status");
    assert!(
        status["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("operation log unreadable")
    );
    assert!(
        error.contains("failed to read log"),
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

    assert_eq!(log["truncated"], true);
    assert_eq!(log["data"]["omitted_lines"], 2);
    assert!(
        log["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("older log line")
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["id"], "op-2");
    assert_eq!(entries[1]["id"], "op-3");
}

#[test]
fn status_reports_omitted_recent_operations() {
    let temp = init_git_repo();
    let root = temp.path();
    write_file(root, "README.md", "# demo\n");
    commit_all(root, "initial commit");

    let mut log = String::new();
    for index in 0..11 {
        log.push_str(&format!(
            "{{\"id\":\"op-{index}\",\"timestamp_unix_ms\":{index},\"kind\":\"observe\",\"op\":\"status\",\"scope\":\".\",\"summary\":\"entry {index}\",\"transaction_id\":null}}\n"
        ));
    }
    write_file(root, ".workspace/log.jsonl", &log);

    let status = run_workspace(root, &["status", "--json"]);
    let entries = status["data"]["recent_operations"]
        .as_array()
        .expect("recent operations should be an array");

    assert_eq!(status["kind"], "workspace_status");
    assert_eq!(status["truncated"], true);
    assert_eq!(status["data"]["recent_operations_omitted"], 1);
    assert_eq!(entries.len(), 10);
    assert_eq!(entries[0]["id"], "op-1");
    assert!(
        status["summary"]
            .as_str()
            .expect("summary should be a string")
            .contains("recent operations truncated")
    );
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
