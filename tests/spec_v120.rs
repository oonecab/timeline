//! v1.2.0 intent annotation: §9's acceptance tests plus the §3.1
//! output contract (only `tag` echoes into context output; `note` never does).

use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use timeline::service::{call_storage, dispatch};
use timeline::storage::Storage;
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
    let id = restored
        .append("tl0", "assistant", "你好", Some(2))
        .unwrap();
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
        .append_with_intent(
            &tl,
            "assistant",
            "好的，开始换",
            Some(5),
            intent("执行", None),
        )
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
    db.summarize(
        &tl,
        &first,
        &second,
        "决定换 Redis 并执行",
        Some(3),
        vec![],
        None,
    )
    .unwrap();
    let t = db.timeline(&tl).unwrap();
    for (id, old) in [first.clone(), second.clone()].iter().zip(before.iter()) {
        let now = serde_json::to_value(t.node(id).unwrap().intent.clone()).unwrap();
        assert_eq!(&now, old, "intent of {id} changed");
    }
    assert_eq!(
        t.node(&first).unwrap().intent,
        intent("决策", Some("出于性能"))
    );
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
    let plain = db
        .append(&tl, "assistant", "我们一步步来", Some(30))
        .unwrap();
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
    assert!(
        dispatch(
            &mut db,
            "timeline_append",
            &json!({"timeline_id": tl, "role": "user", "text": "x", "intent": {"tag": ""}}),
        )
        .is_err()
    );
    assert!(
        dispatch(
            &mut db,
            "timeline_append",
            &json!({"timeline_id": tl, "role": "user", "text": "x",
                "intent": {"tag": "任何词表外的词"}}),
        )
        .is_ok()
    );
    assert!(
        db.append_with_intent(&tl, "user", "x", None, intent("", None))
            .is_err()
    );
}

#[test]
fn summary_deduplication_distinguishes_intent_before_and_after_reload() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let first = db.append(&tl, "user", "original", Some(10)).unwrap();
    let variants = [
        None,
        intent("decision", None),
        intent("decision", Some("")),
        intent("decision", Some("first note")),
        intent("decision", Some("second note")),
        intent("report", Some("first note")),
    ];
    let mut ids = Vec::new();
    let mut unique = HashSet::new();
    for annotation in &variants {
        let id = db
            .summarize_with_intent(
                &tl,
                &first,
                &first,
                "same summary",
                Some(2),
                vec![],
                None,
                annotation.clone(),
            )
            .unwrap();
        assert!(unique.insert(id.clone()), "different intent reused {id}");
        assert_eq!(
            &db.timeline(&tl).unwrap().node(&id).unwrap().intent,
            annotation
        );
        ids.push(id);
    }
    // Check both the live index and the index reconstructed from a saved DB.
    for reload in [false, true] {
        if reload {
            db = serde_json::from_value(serde_json::to_value(&db).unwrap()).unwrap();
            db.rebuild_and_validate().unwrap();
        }
        let before = serde_json::to_value(&db).unwrap();
        for (annotation, expected_id) in variants.iter().zip(&ids) {
            let id = db
                .summarize_with_intent(
                    &tl,
                    &first,
                    &first,
                    "same summary",
                    Some(2),
                    vec![],
                    None,
                    annotation.clone(),
                )
                .unwrap();
            assert_eq!(&id, expected_id);
            assert_eq!(
                &db.timeline(&tl).unwrap().node(&id).unwrap().intent,
                annotation
            );
        }
        assert_eq!(serde_json::to_value(&db).unwrap(), before);
    }
}

#[test]
fn invalid_persisted_intent_rejects_hot_and_cold_data_without_overwriting() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let hot = db.append(&tl, "user", "shared", Some(10)).unwrap();
    db.timeline_mut(&tl).unwrap().fork(&hot, "live").unwrap();
    let cold = db
        .append_with_intent(
            &tl,
            "assistant",
            "archived",
            Some(10),
            intent("report", None),
        )
        .unwrap();
    db.summarize(&tl, &hot, &cold, "covered", Some(1), vec![], None)
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("live").unwrap();
    db.archive_branch(&tl, "main").unwrap();
    db.rebuild_and_validate().unwrap();
    assert!(db.timeline(&tl).unwrap().archive.contains_key(&cold));
    let valid = serde_json::to_value(&db).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("intent.json");
    let backup = directory.path().join("intent.json.bak");
    let backup_bytes = serde_json::to_vec(&valid).unwrap();
    fs::write(&backup, &backup_bytes).unwrap();
    for (section, id) in [("nodes", &hot), ("archive", &cold)] {
        for annotation in [
            json!({"tag":""}),
            json!({"tag":7}),
            json!({"note":"missing tag"}),
            json!({"tag":"report","note":42}),
        ] {
            let mut damaged = valid.clone();
            damaged["timelines"][&tl][section][id]["intent"] = annotation;
            let bytes = serde_json::to_vec(&damaged).unwrap();
            fs::write(&path, &bytes).unwrap();
            let storage = Storage::new(&path);
            assert!(storage.read().is_err(), "accepted invalid {section} intent");
            assert!(
                storage
                    .transaction(&tl, None, |_| {
                        panic!("invalid source must be rejected before invoking a mutation")
                    })
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert_eq!(fs::read(&backup).unwrap(), backup_bytes);
        }
    }
}

#[test]
fn root_and_placeholder_annotations_are_rejected() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let first = db.append(&tl, "user", "long first", Some(100)).unwrap();
    db.append(&tl, "assistant", "long second", Some(100))
        .unwrap();
    let last = db.append(&tl, "user", "target", Some(1)).unwrap();
    let budget = db.context_budget(&tl, &last, 10).unwrap();
    let placeholder = budget.summaries_created.first().unwrap();
    let root = db.timeline(&tl).unwrap().path(&first).unwrap()[0].clone();
    for id in [&root, placeholder] {
        let mut damaged = db.clone();
        damaged
            .timeline_mut(&tl)
            .unwrap()
            .nodes
            .get_mut(id)
            .unwrap()
            .intent = intent("report", None);
        assert!(damaged.rebuild_and_validate().is_err());
    }
    // User-supplied text matching a placeholder must not create an invalid node either.
    let node = db.timeline(&tl).unwrap().node(placeholder).unwrap().clone();
    let range = node.summary_range.unwrap();
    let message = node.message.unwrap();
    let before = serde_json::to_value(&db).unwrap();
    assert!(
        db.summarize_with_intent(
            &tl,
            &range.from,
            &range.to,
            &message.text,
            Some(message.tokens),
            vec![],
            None,
            intent("report", None),
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn intent_survives_storage_roundtrip_without_persisting_output_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("intent.json");
    let storage = Storage::new(&path);
    let tl = call_storage(&storage, "timeline_create", &json!({})).unwrap()["timeline_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let node = call_storage(
        &storage,
        "timeline_append",
        &json!({
            "timeline_id":tl,"role":"user","text":"synthetic message","tokens":10,
            "intent":{"tag":"decision","note":"management only"}
        }),
    )
    .unwrap()["node"]
        .as_str()
        .unwrap()
        .to_owned();
    let reopened = Storage::new(&path);
    let context = call_storage(
        &reopened,
        "timeline_context",
        &json!({"timeline_id":tl,"node":node}),
    )
    .unwrap();
    let budget = call_storage(
        &reopened,
        "timeline_context_budget",
        &json!({"timeline_id":tl,"node":node,"max_tokens":100}),
    )
    .unwrap();
    let expected =
        json!({"role":"user","text":"synthetic message","tokens":10,"intent_tag":"decision"});
    assert_eq!(context, json!([expected.clone()]));
    assert_eq!(budget["messages"], json!([expected]));
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(
        saved["timelines"][&tl]["nodes"][&node]["message"]
            .get("intent_tag")
            .is_none()
    );
    assert_eq!(
        saved["timelines"][&tl]["nodes"][&node]["intent"],
        json!({"tag":"decision","note":"management only"})
    );
}
