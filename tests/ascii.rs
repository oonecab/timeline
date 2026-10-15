use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use timeline::{Database, ascii, drawio};

fn fixture() -> (Database, String) {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let goal = db.append(&tl, "user", "Goal", None).unwrap();
    db.timeline_mut(&tl).unwrap().pin(&goal).unwrap();
    db.append(&tl, "assistant", "Plan A", None).unwrap();
    db.append(&tl, "assistant", "Tests passed", None).unwrap();
    db.timeline_mut(&tl)
        .unwrap()
        .fork(&goal, "alternative")
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("alternative").unwrap();
    db.append(&tl, "assistant", "Plan B", None).unwrap();
    db.append(&tl, "assistant", "Experiment", None).unwrap();
    db.timeline_mut(&tl).unwrap().switch("main").unwrap();
    (db, tl)
}

#[test]
fn readable_branch_tree_is_deterministic_and_keeps_shared_prefix_once() {
    let (db, tl) = fixture();
    let timeline = db.timeline(&tl).unwrap();
    let before = serde_json::to_value(timeline).unwrap();
    let diagram = ascii::render(timeline).unwrap();
    let expected = concat!(
        "Timeline tl0 | current=\"main\"\n",
        "History: parent links | HEAD*: current branch | COLD: archived\n",
        "Summary links are annotations, not merges.\n\n",
        "[n0 root]\n|\n[n1 user PIN] \"Goal\"\n|\n",
        "+-- [n2 assistant] \"Plan A\"\n|   |\n",
        "|   [n3 assistant] \"Tests passed\" [HEAD* \"main\"]\n",
        "`-- [n4 assistant] \"Plan B\"\n    |\n",
        "    [n5 assistant] \"Experiment\" [HEAD \"alternative\"]\n",
    );
    assert_eq!(diagram.text, expected);
    assert!(diagram.text.is_ascii());
    assert_eq!((diagram.nodes, diagram.edges), (6, 5));
    assert_eq!(serde_json::to_value(timeline).unwrap(), before);
    let mut reordered = timeline.clone();
    let mut nodes: Vec<_> = reordered.nodes.drain().collect();
    nodes.sort_by(|a, b| b.0.cmp(&a.0));
    reordered.nodes.extend(nodes);
    assert_eq!(ascii::render(&reordered).unwrap().text, expected);
}

#[test]
fn current_path_precedes_other_branches_and_multiple_heads_stay_on_one_node() {
    let (mut db, tl) = fixture();
    let timeline = db.timeline_mut(&tl).unwrap();
    timeline.fork("n5", "also-here").unwrap();
    timeline.switch("alternative").unwrap();
    let diagram = ascii::render(timeline).unwrap();
    assert!(diagram.text.find("[n4 ").unwrap() < diagram.text.find("[n2 ").unwrap());
    assert!(
        diagram
            .text
            .contains("[HEAD \"also-here\"] [HEAD* \"alternative\"]")
    );
    assert_eq!(diagram.text.matches("[n5 ").count(), 1);
    assert_eq!((diagram.nodes, diagram.edges), (6, 5));
}

#[test]
fn archived_history_and_summary_versions_are_explicit_without_merge_edges() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    db.timeline_mut(&tl).unwrap().fork("n0", "old").unwrap();
    db.timeline_mut(&tl).unwrap().switch("old").unwrap();
    let a = db.append(&tl, "user", "Original A", None).unwrap();
    let b = db.append(&tl, "assistant", "Original B", None).unwrap();
    db.summarize(&tl, &a, &a, "First", None, vec![], None)
        .unwrap();
    let old = db
        .summarize(&tl, &b, &b, "Old", None, vec![], None)
        .unwrap();
    let dependent = db
        .summarize(&tl, &a, &b, "Dependent", None, vec![old.clone()], None)
        .unwrap();
    let latest = db
        .summarize(&tl, &b, &b, "Revised", None, vec![], Some(old.clone()))
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("main").unwrap();
    db.archive_branch(&tl, "old").unwrap();
    let timeline = db.timeline(&tl).unwrap();
    let before = serde_json::to_value(&db).unwrap();
    let diagram = ascii::render(timeline).unwrap();
    assert!(diagram.text.contains(&format!("[{a} user COLD]")));
    assert!(diagram.text.contains(&format!("[{b} assistant COLD]")));
    assert!(
        diagram
            .text
            .contains(&format!("[{dependent} summary v1 STALE]"))
    );
    assert!(diagram.text.contains(&format!("[{latest} summary v2]")));
    assert!(
        diagram
            .text
            .contains(&format!("covers={b}..{b} supersedes={old}"))
    );
    assert!(
        diagram
            .text
            .contains(&format!("covers={a}..{b} derived_from={old}"))
    );
    for node in timeline.nodes.values().chain(timeline.archive.values()) {
        assert_eq!(diagram.text.matches(&format!("[{} ", node.id)).count(), 1);
    }
    let editable = drawio::render(timeline).unwrap();
    assert_eq!(
        (diagram.nodes, diagram.edges),
        (editable.nodes, editable.edges)
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn utf8_previews_escape_terminal_controls_and_are_bounded() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let message = "中文😀\n\r\t\u{1b}[2J\x08\0\u{85}\u{202e}\u{2066}\u{61c}\\\"";
    let id = db.append(&tl, "user", message, None).unwrap();
    let branch = "分支\u{1b}]0;title\x07\r\n";
    db.timeline_mut(&tl).unwrap().fork(&id, branch).unwrap();
    db.timeline_mut(&tl).unwrap().switch(branch).unwrap();
    db.append(&tl, "assistant", &"长消息".repeat(50_000), None)
        .unwrap();
    let diagram = ascii::render(db.timeline(&tl).unwrap()).unwrap();
    assert!(diagram.text.contains("中文😀\\n\\r\\t\\u{1b}[2J"));
    assert!(diagram.text.contains("\\u{202e}\\u{2066}\\u{61c}"));
    assert!(diagram.text.contains("\\\\\\\""));
    assert!(diagram.text.contains("...\""));
    assert!(diagram.text.chars().all(|c| !c.is_control() || c == '\n'));
    assert!(!diagram.text.contains('\u{202e}'));
    assert!(diagram.text.len() < 2_000);
}

#[test]
fn empty_and_long_linear_history_need_no_recursion_or_growing_indentation() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let empty = ascii::render(db.timeline(&tl).unwrap()).unwrap();
    assert_eq!((empty.nodes, empty.edges), (1, 0));
    assert!(empty.text.contains("[n0 root] [HEAD* \"main\"]"));
    for _ in 0..6_000 {
        db.append(&tl, "user", "message", Some(1)).unwrap();
    }
    let diagram = ascii::render(db.timeline(&tl).unwrap()).unwrap();
    assert_eq!((diagram.nodes, diagram.edges), (6_001, 6_000));
    assert!(diagram.text.lines().all(|line| !line.starts_with(' ')));
    assert!(diagram.text.len() < 300_000);
    assert!(
        diagram
            .text
            .contains("[n6000 user] \"message\" [HEAD* \"main\"]")
    );
}

#[test]
fn placeholders_are_marked_without_mutating_the_timeline() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    for _ in 0..4 {
        db.append(&tl, "user", "history", Some(100)).unwrap();
    }
    let head = db.append(&tl, "assistant", "now", Some(1)).unwrap();
    let budget = db.context_budget(&tl, &head, 33).unwrap();
    let before = serde_json::to_value(&db).unwrap();
    let diagram = ascii::render(db.timeline(&tl).unwrap()).unwrap();
    for id in budget.summaries_created {
        assert!(
            diagram
                .text
                .contains(&format!("[{id} summary v1 PLACEHOLDER]"))
        );
    }
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

fn cli(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tl"))
        .arg("--db")
        .arg(path)
        .args(args)
        .env_remove("TIMELINE_DB")
        .output()
        .unwrap()
}

fn failure(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    error["error"].as_str().unwrap().to_owned()
}

#[test]
fn cli_prints_clean_text_and_writes_the_same_utf8_file_without_touching_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.json");
    let backup = temp.path().join("db.json.bak");
    let (mut db, tl) = fixture();
    db.append(&tl, "user", "终端可查看中文", None).unwrap();
    let original = serde_json::to_vec(&db).unwrap();
    fs::write(&path, &original).unwrap();
    fs::write(&backup, &original).unwrap();
    let expected = ascii::render(db.timeline(&tl).unwrap()).unwrap();
    let stdout = cli(&path, &["export", "--format", "ascii"]);
    assert!(stdout.status.success());
    assert!(stdout.stderr.is_empty());
    assert_eq!(stdout.stdout, expected.text.as_bytes());
    let output = temp.path().join("中文 节点.txt");
    let args = [
        "export",
        "--format",
        "ascii",
        "--output",
        output.to_str().unwrap(),
    ];
    let saved = cli(&path, &args);
    assert!(saved.status.success());
    assert!(saved.stderr.is_empty());
    let result: Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(result["format"], "ascii");
    assert_eq!(result["timeline_id"], tl);
    assert_eq!(result["nodes"], expected.nodes);
    assert_eq!(result["edges"], expected.edges);
    assert_eq!(fs::read(&output).unwrap(), stdout.stdout);
    fs::write(&output, "manual notes").unwrap();
    assert!(failure(cli(&path, &args)).contains("already exists"));
    assert_eq!(fs::read_to_string(&output).unwrap(), "manual notes");
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(&backup).unwrap(), original);
}

#[test]
fn cli_errors_are_json_and_drawio_still_requires_an_output_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.json");
    fs::write(&path, serde_json::to_vec(&fixture().0).unwrap()).unwrap();
    for args in [vec!["export"], vec!["export", "--format", "drawio"]] {
        let output = cli(&path, &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(failure(output).contains("--output"));
    }
    assert!(
        failure(cli(
            &path,
            &["--idempotency-key", "x", "export", "--format", "ascii"]
        ))
        .contains("read-only")
    );
    assert!(
        failure(cli(
            &path,
            &["--timeline", "tl999", "export", "--format", "ascii"]
        ))
        .contains("timeline")
    );
    assert!(
        failure(cli(
            &path,
            &[
                "export",
                "--format",
                "ascii",
                "--output",
                path.to_str().unwrap()
            ]
        ))
        .contains("already exists")
    );
    fs::write(&path, b"corrupt").unwrap();
    failure(cli(&path, &["export", "--format", "ascii"]));
    assert_eq!(fs::read(&path).unwrap(), b"corrupt");
}

#[test]
fn closed_stdout_pipe_exits_cleanly_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.json");
    let mut db = Database::default();
    let tl = db.create().unwrap();
    for _ in 0..3_000 {
        db.append(&tl, "user", "message", Some(1)).unwrap();
    }
    fs::write(&path, serde_json::to_vec(&db).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tl"))
        .arg("--db")
        .arg(&path)
        .args(["export", "--format", "ascii"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}
