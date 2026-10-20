//! v1.2.0 intent annotation: §9's two new acceptance tests plus the §A(a)
//! output contract (only `tag` echoes into context output; `note` never does).

use serde_json::{Value, json};
use timeline::service::dispatch;
use timeline::{Database, Intent};

fn intent(tag: &str, note: Option<&str>) -> Option<Intent> {
    Some(Intent {
        tag: tag.into(),
        note: note.map(str::to_owned),
    })
}

#[test]
fn axiom_intent_optional() {
    // A pre-v1.2 database: node JSON has no `intent` field at all.
    let legacy = json!({
        "format_version": 1,
        "next_timeline": 1,
        "next_node": 2,
        "timelines": {
            "tl0": {
                "id": "tl0",
                "nodes": {
                    "n0": {"id":"n0", "parent":null, "children":["n1"],
                        "message":null, "pinned":false, "prefix_tokens":0, "summary_range":null},
                    "n1": {"id":"n1", "parent":"n0", "children":[],
                        "message":{"role":"user", "text":"你好", "tokens":5},
                        "pinned":false, "prefix_tokens":5, "summary_range":null}
                },
                "branches":{"main":"n1"},
                "current_branch":"main"
            }
        }
    });
    let mut db: Database = serde_json::from_value(legacy).unwrap();
    db.rebuild_and_validate().unwrap();
    assert_eq!(db.timeline("tl0").unwrap().node("n1").unwrap().intent, None);
    // Round-trip: serialize and read back; intent stays absent.
    let text = serde_json::to_string(&db).unwrap();
    let mut restored: Database = serde_json::from_str(&text).unwrap();
    restored.rebuild_and_validate().unwrap();
    assert_eq!(
        restored.timeline("tl0").unwrap().node("n1").unwrap().intent,
        None
    );
    // New appends on the legacy timeline default to no intent.
    let id = restored.append("tl0", "assistant", "你好", Some(2)).unwrap();
    assert_eq!(
        restored.timeline("tl0").unwrap().node(&id).unwrap().intent,
        None
    );
}

#[test]
fn axiom_intent_immutable() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let first = db
        .append_with_intent(
            &tl,
            "user",
            "把缓存换成 Redis",
            Some(6),
            intent("决策", Some("出于性能")),
        )
        .unwrap();
    let second = db
        .append_with_intent(&tl, "assistant", "好的，开始换", Some(5), intent("执行", None))
        .unwrap();
    let before: Vec<Value> = [first.clone(), second.clone()]
        .iter()
        .map(|id| {
            serde_json::to_value(db.timeline(&tl).unwrap().node(id).unwrap().intent.clone())
                .unwrap()
        })
        .collect();
    // Later operations must not touch existing annotations (invariant 5).
    db.append(&tl, "user", "顺便加监控", Some(4)).unwrap();
    db.timeline_mut(&tl).unwrap().fork(&first, "alt").unwrap();
    db.timeline_mut(&tl).unwrap().pin(&second).unwrap();
    db.summarize(&tl, &first, &second, "决定换 Redis 并执行", Some(3), vec![], None)
        .unwrap();
    let t = db.timeline(&tl).unwrap();
    for (id, old) in [first.clone(), second.clone()].iter().zip(before.iter()) {
        let now = serde_json::to_value(t.node(id).unwrap().intent.clone()).unwrap();
        assert_eq!(&now, old, "intent of {id} changed");
    }
    assert_eq!(t.node(&first).unwrap().intent, intent("决策", Some("出于性能")));
    assert_eq!(t.node(&second).unwrap().intent, intent("执行", None));
}

#[test]
fn intent_tag_in_context_and_budget_output() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let asked = db
        .append_with_intent(
            &tl,
            "user",
            "引理2看不懂",
            Some(20),
            intent("提问", Some("请求解释")),
        )
        .unwrap();
    let plain = db.append(&tl, "assistant", "我们一步步来", Some(30)).unwrap();
    let t = db.timeline(&tl).unwrap();
    let messages = t.context(&plain).unwrap();
    assert_eq!(messages[0].intent_tag.as_deref(), Some("提问"));
    assert_eq!(messages[1].intent_tag, None);
    let result = db.context_budget(&tl, &plain, 1000).unwrap();
    assert_eq!(result.messages[0].intent_tag.as_deref(), Some("提问"));
    assert_eq!(result.messages[1].intent_tag, None);
    // Transport JSON: context exposes only the tag, never the note, and
    // omits the field entirely for unannotated messages.
    let out = dispatch(
        &mut db,
        "timeline_context",
        &json!({"timeline_id": tl, "node": plain}),
    )
    .unwrap();
    assert_eq!(out[0]["intent_tag"], "提问");
    assert!(out[0].get("note").is_none());
    assert!(out[1].get("intent_tag").is_none());
    // timeline_node exposes the full annotation for management decisions.
    let node = dispatch(
        &mut db,
        "timeline_node",
        &json!({"timeline_id": tl, "node": asked}),
    )
    .unwrap();
    assert_eq!(node["intent"], json!({"tag": "提问", "note": "请求解释"}));
}

#[test]
fn intent_via_dispatch_and_validation() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let created = dispatch(
        &mut db,
        "timeline_append",
        &json!({"timeline_id": tl, "role": "user", "text": "开始",
                "intent": {"tag": "指令", "note": "启动任务"}}),
    )
    .unwrap();
    let id = created["node"].as_str().unwrap().to_owned();
    assert_eq!(
        db.timeline(&tl).unwrap().node(&id).unwrap().intent,
        intent("指令", Some("启动任务"))
    );
    // Summaries may carry intent too; placeholders never do.
    let summary = dispatch(
        &mut db,
        "timeline_summarize",
        &json!({"timeline_id": tl, "from": id, "to": id, "summary": "启动",
                "intent": {"tag": "摘要"}}),
    )
    .unwrap();
    let summary_id = summary["node"].as_str().unwrap().to_owned();
    assert_eq!(
        db.timeline(&tl).unwrap().node(&summary_id).unwrap().intent,
        intent("摘要", None)
    );
    // Empty tag is rejected at both layers; unknown tags are accepted
    // (spec §10: the vocabulary belongs to the skill layer, not the spec).
    assert!(dispatch(
        &mut db,
        "timeline_append",
        &json!({"timeline_id": tl, "role": "user", "text": "x", "intent": {"tag": ""}}),
    )
    .is_err());
    assert!(dispatch(
        &mut db,
        "timeline_append",
        &json!({"timeline_id": tl, "role": "user", "text": "x",
                "intent": {"tag": "任何词表外的词"}}),
    )
    .is_ok());
    assert!(db
        .append_with_intent(&tl, "user", "x", None, intent("", None))
        .is_err());
}
