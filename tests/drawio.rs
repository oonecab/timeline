use roxmltree::{Document, Node};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use timeline::{Database, drawio};

fn object<'a, 'input>(doc: &'a Document<'input>, id: &str) -> Node<'a, 'input> {
    doc.descendants()
        .find(|n| n.attribute("id") == Some(id))
        .unwrap()
}

fn cell<'a, 'input>(node: Node<'a, 'input>) -> Node<'a, 'input> {
    node.children().find(|n| n.has_tag_name("mxCell")).unwrap()
}

fn position(node: Node<'_, '_>) -> (u64, u64, u64, u64) {
    let geometry = cell(node)
        .children()
        .find(|n| n.has_tag_name("mxGeometry"))
        .unwrap();
    let number = |key| geometry.attribute(key).unwrap().parse().unwrap();
    (number("x"), number("y"), number("width"), number("height"))
}

fn fixture() -> (Database, String, String, String, String, String) {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let pin = db.append(&tl, "user", "只使用本地数据", None).unwrap();
    let from = db
        .append(&tl, "assistant", "方案 A：内存索引", None)
        .unwrap();
    let head = db
        .append(&tl, "assistant", "记录样本文档检索结果", None)
        .unwrap();
    db.timeline_mut(&tl).unwrap().pin(&pin).unwrap();
    db.timeline_mut(&tl)
        .unwrap()
        .fork(&pin, "替代方案")
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("替代方案").unwrap();
    db.append(&tl, "assistant", "方案 B：文件扫描", None)
        .unwrap();
    db.append(&tl, "assistant", "保留探索结果", None).unwrap();
    db.timeline_mut(&tl).unwrap().switch("main").unwrap();
    let old = db
        .summarize(&tl, &from, &head, "初版摘要", Some(10), vec![], None)
        .unwrap();
    let dependent = db
        .summarize(
            &tl,
            &from,
            &from,
            "参考初版",
            Some(5),
            vec![old.clone()],
            None,
        )
        .unwrap();
    let latest = db
        .summarize(
            &tl,
            &from,
            &head,
            "结论：采用内存索引。\n依据：样本结果。\n未决：性能验证。",
            Some(12),
            vec![],
            Some(old.clone()),
        )
        .unwrap();
    (db, tl, old, latest, dependent, head)
}

#[test]
fn native_cells_preserve_history_metadata_and_distinct_summary_relations() {
    let (db, tl, old, latest, dependent, head) = fixture();
    let timeline = db.timeline(&tl).unwrap();
    let before = serde_json::to_value(timeline).unwrap();
    let diagram = drawio::render(timeline).unwrap();
    assert_eq!(serde_json::to_value(timeline).unwrap(), before);
    let doc = Document::parse(&diagram.xml).expect("valid XML 1.0");
    assert_eq!(doc.root_element().tag_name().name(), "mxfile");
    assert_eq!(doc.root_element().attribute("compressed"), Some("false"));
    assert_eq!(object(&doc, "1").attribute("parent"), Some("0"));
    for layer in ["coverage", "versions"] {
        assert_eq!(object(&doc, layer).attribute("visible"), Some("0"));
    }
    let ids: Vec<_> = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    assert_eq!(ids.iter().copied().collect::<HashSet<_>>().len(), ids.len());
    let vertices: Vec<_> = doc
        .descendants()
        .filter(|n| n.attribute("node_id").is_some())
        .collect();
    assert_eq!(vertices.len(), timeline.nodes.len());
    assert_eq!(diagram.nodes, vertices.len());
    for node in timeline.nodes.values() {
        let item = object(&doc, &node.id);
        assert_eq!(cell(item).attribute("vertex"), Some("1"));
        assert!(cell(item).attribute("style").unwrap().contains("html=0;"));
        assert_eq!(item.attribute("timeline_id"), Some(tl.as_str()));
        let text: String = serde_json::from_str(item.attribute("text_json").unwrap()).unwrap();
        assert_eq!(text, node.message.as_ref().map_or("", |m| m.text.as_str()));
        assert_eq!(item.attribute("zone"), Some("hot"));
        assert_eq!(
            item.attribute("pinned"),
            Some(if node.pinned { "true" } else { "false" })
        );
        if let Some(parent) = &node.parent {
            let edge = cell(object(&doc, &format!("parent-{}", node.id)));
            assert_eq!(edge.attribute("source"), Some(parent.as_str()));
            assert_eq!(edge.attribute("target"), Some(node.id.as_str()));
        }
    }
    let parent_count = doc
        .descendants()
        .filter(|n| n.attribute("relation") == Some("parent"))
        .count();
    assert_eq!(
        parent_count,
        diagram.nodes - 1,
        "no invented merges or duplicated shared prefixes"
    );
    for edge in doc
        .descendants()
        .filter(|n| n.attribute("edge") == Some("1"))
    {
        assert!(ids.contains(&edge.attribute("source").unwrap()));
        assert!(ids.contains(&edge.attribute("target").unwrap()));
    }
    assert_eq!(
        diagram.edges,
        doc.descendants()
            .filter(|n| n.attribute("edge") == Some("1"))
            .count()
    );
    assert_eq!(
        object(&doc, &latest).attribute("supersedes"),
        Some(old.as_str())
    );
    assert_eq!(
        object(&doc, &latest).attribute("summary_version"),
        Some("2")
    );
    assert_eq!(object(&doc, &dependent).attribute("stale"), Some("true"));
    let revision = cell(object(&doc, &format!("supersedes-{latest}")));
    assert_eq!(revision.attribute("source"), Some(latest.as_str()));
    assert_eq!(revision.attribute("target"), Some(old.as_str()));
    assert_eq!(revision.attribute("parent"), Some("versions"));
    assert_eq!(
        cell(object(&doc, &format!("coverage-to-{latest}"))).attribute("target"),
        Some(head.as_str())
    );
    assert_eq!(
        cell(object(&doc, &format!("derived-{dependent}-0"))).attribute("target"),
        Some(old.as_str())
    );
    assert_eq!(object(&doc, &head).attribute("current_head"), Some("true"));
    assert!(
        cell(object(&doc, &head))
            .attribute("style")
            .unwrap()
            .contains("fillColor=#E8F4EC;")
    );
}

#[test]
fn layout_is_horizontal_nonoverlapping_and_deterministic() {
    let (db, tl, _, _, _, head) = fixture();
    let timeline = db.timeline(&tl).unwrap();
    let first = drawio::render(timeline).unwrap().xml;
    let mut reordered = timeline.clone();
    let mut values: Vec<_> = reordered.nodes.drain().collect();
    values.sort_by(|a, b| b.0.cmp(&a.0));
    reordered.nodes.extend(values);
    assert_eq!(first, drawio::render(&reordered).unwrap().xml);
    let doc = Document::parse(&first).unwrap();
    let positions: BTreeMap<_, _> = timeline
        .nodes
        .keys()
        .map(|id| (id, position(object(&doc, id))))
        .collect();
    let path = timeline.path(&head).unwrap();
    for pair in path.windows(2) {
        let a = positions[&pair[0]];
        let b = positions[&pair[1]];
        assert!(a.0 + a.2 < b.0);
        assert_eq!(a.1, b.1, "current path continues horizontally");
    }
    for node in timeline.nodes.values() {
        if let Some(parent) = &node.parent {
            assert!(positions[parent].0 < positions[&node.id].0);
        }
    }
    for (a_id, &(ax, ay, aw, ah)) in &positions {
        for (b_id, &(bx, by, bw, bh)) in &positions {
            if a_id != b_id {
                assert!(
                    ax + aw <= bx || bx + bw <= ax || ay + ah <= by || by + bh <= ay,
                    "{a_id} overlaps {b_id}"
                );
            }
        }
    }
}

#[test]
fn cold_nodes_and_hot_summaries_remain_connected_after_archive() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let root = db.timeline(&tl).unwrap().branches["main"].clone();
    db.timeline_mut(&tl).unwrap().fork(&root, "old").unwrap();
    db.timeline_mut(&tl).unwrap().switch("old").unwrap();
    let a = db.append(&tl, "user", "历史原文", None).unwrap();
    let b = db.append(&tl, "assistant", "旧路线结果", None).unwrap();
    db.summarize(&tl, &a, &a, "第一段", None, vec![], None)
        .unwrap();
    let summary = db
        .summarize(&tl, &b, &b, "第二段", None, vec![], None)
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("main").unwrap();
    db.archive_branch(&tl, "old").unwrap();
    let diagram = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    let doc = Document::parse(&diagram.xml).unwrap();
    assert_eq!(object(&doc, &a).attribute("zone"), Some("cold"));
    assert_eq!(object(&doc, &b).attribute("zone"), Some("cold"));
    assert_eq!(object(&doc, &summary).attribute("zone"), Some("hot"));
    assert_eq!(
        object(&doc, &summary).attribute("parent_id"),
        Some(a.as_str())
    );
    assert_eq!(
        cell(object(&doc, &format!("parent-{summary}"))).attribute("source"),
        Some(a.as_str())
    );
    assert_eq!(
        cell(object(&doc, &format!("coverage-from-{summary}"))).attribute("target"),
        Some(b.as_str())
    );
    assert_eq!(diagram.nodes, 5);
}

#[test]
fn arbitrary_text_round_trips_without_becoming_markup_or_placeholders() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let text = "中文😀 <script>alert(1)</script> & \"quotes\" 'apostrophe'\n\r\t\0\u{1f}\u{fffe}\u{ffff} %date% \\uffff";
    let id = db.append(&tl, "user", text, None).unwrap();
    let branch = "分支 & \"测试\" <b> %name% \u{ffff}";
    db.timeline_mut(&tl).unwrap().fork(&id, branch).unwrap();
    db.timeline_mut(&tl).unwrap().switch(branch).unwrap();
    let diagram = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    let doc = Document::parse(&diagram.xml).unwrap();
    let item = object(&doc, &id);
    assert_eq!(
        serde_json::from_str::<String>(item.attribute("text_json").unwrap()).unwrap(),
        text
    );
    let branches: Vec<String> =
        serde_json::from_str(item.attribute("branches_json").unwrap()).unwrap();
    assert!(branches.iter().any(|b| b == branch));
    assert_eq!(item.attribute("placeholders"), Some("0"));
    assert!(!doc.descendants().any(|n| n.has_tag_name("script")));
    assert!(!doc.descendants().any(|n| n.attribute("link").is_some()));
    assert!(cell(item).attribute("style").unwrap().contains("html=0;"));
}

#[test]
fn long_messages_keep_complete_text_and_bounded_card_labels() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let text = "很长的消息<&>😀".repeat(20_000);
    let id = db.append(&tl, "assistant", &text, None).unwrap();
    let diagram = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    let doc = Document::parse(&diagram.xml).unwrap();
    let item = object(&doc, &id);
    assert!(item.attribute("label").unwrap().chars().count() < 150);
    assert!(item.attribute("label").unwrap().contains('…'));
    assert_eq!(
        serde_json::from_str::<String>(item.attribute("text_json").unwrap()).unwrap(),
        text
    );
}

#[test]
fn empty_and_long_linear_timelines_export_without_recursion() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let empty = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    assert_eq!((empty.nodes, empty.edges), (1, 0));
    Document::parse(&empty.xml).unwrap();
    let mut head = String::new();
    for _ in 0..6_000 {
        head = db.append(&tl, "user", "消息", Some(1)).unwrap();
    }
    let diagram = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    assert_eq!((diagram.nodes, diagram.edges), (6_001, 6_000));
    let doc = Document::parse(&diagram.xml).unwrap();
    assert_eq!(
        position(object(&doc, "n0")).1,
        position(object(&doc, &head)).1
    );
}

#[test]
fn budget_placeholders_are_exported_without_filling_or_creating_more() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    for _ in 0..4 {
        db.append(&tl, "user", "历史", Some(100)).unwrap();
    }
    let head = db.append(&tl, "assistant", "当前", Some(1)).unwrap();
    let budget = db.context_budget(&tl, &head, 33).unwrap();
    assert!(!budget.summaries_created.is_empty());
    let before = serde_json::to_value(&db).unwrap();
    let diagram = drawio::render(db.timeline(&tl).unwrap()).unwrap();
    let doc = Document::parse(&diagram.xml).unwrap();
    for id in budget.summaries_created {
        assert_eq!(object(&doc, &id).attribute("placeholder"), Some("true"));
    }
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

fn cli(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tl"))
        .arg("--db")
        .arg(db)
        .args(args)
        .env_remove("TIMELINE_DB")
        .output()
        .unwrap()
}

fn write_fixture(path: &Path) {
    fs::write(path, serde_json::to_vec(&fixture().0).unwrap()).unwrap();
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn failure(output: Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    result["error"].as_str().unwrap().to_owned()
}

#[test]
fn cli_export_and_manual_diagram_edits_never_change_source_or_backup() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.json");
    write_fixture(&path);
    let backup = temp.path().join("db.json.bak");
    fs::copy(&path, &backup).unwrap();
    let before = fs::read(&path).unwrap();
    let output = temp.path().join("中文 节点.drawio");
    let response = success(cli(
        &path,
        &[
            "--timeline",
            "tl0",
            "export",
            "--format",
            "drawio",
            "--output",
            output.to_str().unwrap(),
        ],
    ));
    assert_eq!(response["format"], "drawio");
    assert_eq!(response["timeline_id"], "tl0");
    let xml = fs::read_to_string(&output).unwrap();
    let doc = Document::parse(&xml).unwrap();
    assert_eq!(
        response["nodes"],
        doc.descendants()
            .filter(|n| n.attribute("node_id").is_some())
            .count()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&backup).unwrap(), before);
    let edited = xml
        .replace("x=\"40\"", "x=\"75\"")
        .replace("只使用本地数据", "用户在图上的批注");
    fs::write(&output, &edited).unwrap();
    assert!(
        failure(cli(
            &path,
            &["export", "--output", output.to_str().unwrap()]
        ))
        .contains("already exists")
    );
    assert_eq!(fs::read_to_string(&output).unwrap(), edited);
    let node = success(cli(&path, &["node", "--node", "n1"]));
    assert_eq!(node["text"], "只使用本地数据");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&backup).unwrap(), before);
    assert!(!fs::read_dir(temp.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".timeline-export-")
    }));
}

#[test]
fn cli_export_rejects_bad_inputs_and_never_clobbers_database_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.drawio");
    write_fixture(&path);
    let before = fs::read(&path).unwrap();
    let output = temp.path().join("out.drawio");
    let args = ["export", "--output", output.to_str().unwrap()];
    assert!(
        failure(cli(&path, &["export", "--output", path.to_str().unwrap()]))
            .contains("already exists")
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        failure(cli(
            &path,
            &["--timeline", "tl999", args[0], args[1], args[2]]
        ))
        .contains("timeline")
    );
    assert!(!output.exists());
    assert!(
        failure(cli(
            &path,
            &["export", "--format", "svg", "--output", args[2]]
        ))
        .contains("invalid value")
    );
    assert!(
        failure(cli(
            &path,
            &["--idempotency-key", "k", args[0], args[1], args[2]]
        ))
        .contains("read-only")
    );
    let missing_dir = temp.path().join("missing").join("out.drawio");
    assert!(
        failure(cli(
            &path,
            &["export", "--output", missing_dir.to_str().unwrap()]
        ))
        .contains("directory")
    );
    assert!(!missing_dir.exists());
    let missing_db = temp.path().join("absent.json");
    assert!(failure(cli(&missing_db, &args)).contains("no timeline"));
    assert!(!missing_db.exists());
    fs::write(&path, b"corrupt").unwrap();
    failure(cli(&path, &args));
    assert!(!output.exists());
    assert_eq!(fs::read(&path).unwrap(), b"corrupt");
}

#[test]
fn cli_export_respects_environment_and_numeric_default_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("many.json");
    let mut db = Database::default();
    for _ in 0..11 {
        db.create().unwrap();
    }
    db.timelines.retain(|id, _| id == "tl9" || id == "tl10");
    fs::write(&path, serde_json::to_vec(&db).unwrap()).unwrap();
    let output = temp.path().join("earliest.drawio");
    let result = success(
        Command::new(env!("CARGO_BIN_EXE_tl"))
            .env("TIMELINE_DB", &path)
            .args(["export", "--output"])
            .arg(&output)
            .output()
            .unwrap(),
    );
    assert_eq!(result["timeline_id"], "tl9");
    let other = temp.path().join("explicit.drawio");
    let result = success(cli(
        &path,
        &[
            "--timeline",
            "tl10",
            "export",
            "--output",
            other.to_str().unwrap(),
        ],
    ));
    assert_eq!(result["timeline_id"], "tl10");
    assert_eq!(result["nodes"], json!(1));
}

#[test]
fn concurrent_exports_to_one_output_have_exactly_one_winner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.json");
    write_fixture(&path);
    let output = temp.path().join("once.drawio");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_tl"))
            .arg("--db")
            .arg(&path)
            .args(["export", "--output"])
            .arg(&output)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = run();
    let b = run();
    let outputs = [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
    assert_eq!(outputs.iter().filter(|o| o.status.success()).count(), 1);
    Document::parse(&fs::read_to_string(&output).unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn export_refuses_symlink_outputs_without_changing_their_target() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.json");
    write_fixture(&path);
    let link = temp.path().join("alias.drawio");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let before = fs::read(&path).unwrap();
    failure(cli(&path, &["export", "--output", link.to_str().unwrap()]));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
