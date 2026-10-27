//! §9's fifteen acceptance tests, with deterministic property cases.
use serde_json::json;
use timeline::{Database, RESERVE, estimate_tokens, storage::Storage};

fn chain(costs: &[u64]) -> (Database, String, Vec<String>) {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let ids = costs
        .iter()
        .enumerate()
        .map(|(i, &cost)| {
            db.append(&tl, "user", &format!("message-{i}"), Some(cost))
                .unwrap()
        })
        .collect();
    (db, tl, ids)
}
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn axiom_immutability() {
    let (mut db, tl, ids) = chain(&[10, 20, 30]);
    let original = db.timeline(&tl).unwrap().nodes.clone();
    db.append(&tl, "assistant", "continuation", Some(5))
        .unwrap();
    db.timeline_mut(&tl)
        .unwrap()
        .fork(&ids[0], "平行宇宙")
        .unwrap();
    db.timeline_mut(&tl).unwrap().pin(&ids[1]).unwrap();
    let t = db.timeline(&tl).unwrap();
    for (id, old) in original {
        let n = t.node(&id).unwrap();
        assert_eq!(n.message, old.message);
        assert_eq!(n.parent, old.parent);
        assert_eq!(n.prefix_tokens, old.prefix_tokens);
    }
    for id in t.nodes.keys() {
        assert_eq!(
            t.tokens(id).unwrap(),
            t.context(id).unwrap().iter().map(|m| m.tokens).sum::<u64>()
        );
    }
    t.validate().unwrap();
}
#[test]
fn axiom_fork_zero_copy() {
    let (mut db, tl, ids) = chain(&[1, 2]);
    let t = db.timeline_mut(&tl).unwrap();
    let count = t.nodes.len();
    let main = t.context(&ids[1]).unwrap();
    t.fork(&ids[0], "b").unwrap();
    assert_eq!(count, t.nodes.len());
    assert_eq!(t.branches["b"], ids[0]);
    t.switch("b").unwrap();
    db.append(&tl, "assistant", "branch", Some(3)).unwrap();
    assert_eq!(db.timeline(&tl).unwrap().context(&ids[1]).unwrap(), main);
}
#[test]
fn axiom_path_is_context() {
    let (mut db, tl, ids) = chain(&[1, 2, 3]);
    let t = db.timeline_mut(&tl).unwrap();
    t.fork(&ids[0], "b").unwrap();
    t.switch("b").unwrap();
    let branch = db.append(&tl, "assistant", "branch-only", Some(4)).unwrap();
    let t = db.timeline(&tl).unwrap();
    assert_eq!(
        t.context(&branch)
            .unwrap()
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec!["message-0", "branch-only"]
    );
    assert!(t.context("n0").unwrap().is_empty());
    assert_eq!(t.context(&ids[2]).unwrap().len(), 3);
    // v1.2 output shape: messages carry `intent_tag` only for annotated
    // nodes, so unannotated history keeps the pre-v1.2 shape exactly.
    assert!(
        t.context(&branch)
            .unwrap()
            .iter()
            .all(|m| m.intent_tag.is_none())
    );
}
#[test]
fn axiom_branch_isolation() {
    let (mut db, tl, ids) = chain(&[5, 10]);
    let t = db.timeline_mut(&tl).unwrap();
    let old = (
        t.branches["main"].clone(),
        t.context(&ids[1]).unwrap(),
        t.tokens(&ids[1]).unwrap(),
    );
    t.fork(&ids[0], "b").unwrap();
    t.switch("b").unwrap();
    for _ in 0..3 {
        db.append(&tl, "user", "b", Some(9)).unwrap();
    }
    let t = db.timeline(&tl).unwrap();
    assert_eq!(
        (
            t.branches["main"].clone(),
            t.context(&ids[1]).unwrap(),
            t.tokens(&ids[1]).unwrap()
        ),
        old
    );
}
#[test]
fn axiom_budget_bound() {
    let (mut db, tl, ids) = chain(&[50; 10]);
    let result = db.context_budget(&tl, &ids[9], 150).unwrap();
    assert_eq!(result.total_tokens, 182);
    assert_eq!(result.messages.last().unwrap().text, "message-9");
    let mut seed = 0x54494d454c494e45;
    for case in 0..1000 {
        let n = 1 + (next(&mut seed) % 70) as usize;
        let costs: Vec<_> = (0..n).map(|_| next(&mut seed) % 201).collect();
        // Target fits; unrestricted targets and infeasible pinned gaps are
        // independently tested in adversarial.rs, not silently skipped here.
        let max = costs[n - 1] + next(&mut seed) % 2000;
        let (mut db, tl, ids) = chain(&costs);
        let result = db.context_budget(&tl, &ids[n - 1], max).unwrap();
        assert!(result.total_tokens <= max + RESERVE, "case {case}");
        assert_eq!(
            result.messages.last().unwrap().text,
            format!("message-{}", n - 1)
        );
        assert_eq!(
            result.total_tokens,
            result.messages.iter().map(|m| m.tokens).sum::<u64>()
        );
    }
    // v1.1.2 §5.5/§9: mandatory gaps cannot fit in max_tokens + RESERVE.
    let (mut db, tl, ids) = chain(&[1000, 1, 1000, 0]);
    db.timeline_mut(&tl).unwrap().pin(&ids[1]).unwrap();
    let before = serde_json::to_value(&db).unwrap();
    let error = db.context_budget(&tl, &ids[3], 1).unwrap_err();
    assert_eq!(error.message, "budget infeasible");
    assert_eq!(
        error.data,
        Some(json!({"required_tokens":65,"max_tokens":1}))
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}
#[test]
fn axiom_pin_beats_budget() {
    let (mut db, tl, ids) = chain(&[50; 10]);
    db.timeline_mut(&tl).unwrap().pin(&ids[1]).unwrap();
    let r = db.context_budget(&tl, &ids[9], 150).unwrap();
    assert!(r.messages.iter().any(|m| m.text == "message-1"));
    assert!(r.total_tokens <= 182);
    for id in &ids {
        db.timeline_mut(&tl).unwrap().pin(id).unwrap();
    }
    let r = db.context_budget(&tl, &ids[9], 1).unwrap();
    assert_eq!(r.total_tokens, 500);
    assert_eq!(r.messages.len(), 10);
}
#[test]
fn axiom_summarize_reversible() {
    let (mut db, tl, ids) = chain(&[10; 6]);
    let before = db.timeline(&tl).unwrap().context(&ids[4]).unwrap();
    let s = db
        .summarize(&tl, &ids[1], &ids[4], "summary", Some(2), vec![], None)
        .unwrap();
    let t = db.timeline(&tl).unwrap();
    assert_eq!(t.context(&ids[4]).unwrap(), before);
    let n = t.node(&s).unwrap();
    assert_eq!(n.parent.as_ref(), Some(&ids[0]));
    assert_eq!(n.summary_range.as_ref().unwrap().from, ids[1]);
    assert!(t.node(&ids[0]).unwrap().children.contains(&s));
    let audit = db.audit.last().unwrap();
    assert_eq!(audit.node.as_ref(), Some(&s));
    assert_eq!(audit.from.as_ref(), Some(&ids[1]));
    assert_eq!(audit.to.as_ref(), Some(&ids[4]));
}
#[test]
fn axiom_placeholder_dedup() {
    let (mut db, tl, ids) = chain(&[50; 10]);
    let a = db.context_budget(&tl, &ids[9], 150).unwrap();
    let count = db.next_node;
    let audits = db.audit.len();
    let b = db.context_budget(&tl, &ids[9], 150).unwrap();
    assert_eq!(a, b);
    assert_eq!(db.next_node, count);
    assert_eq!(db.audit.len(), audits);
    assert_eq!(audits, a.summaries_created.len());
    for id in a.summaries_created {
        let n = db.timeline(&tl).unwrap().node(&id).unwrap();
        assert!(n.is_placeholder());
        assert_eq!(n.message.as_ref().unwrap().tokens, RESERVE);
        assert!(
            db.audit
                .iter()
                .any(|e| e.op == "placeholder" && e.node.as_ref() == Some(&id))
        );
    }
}
#[test]
fn axiom_priority_tiers() {
    let (mut db, tl, ids) = chain(&[50; 5]);
    db.summarize(
        &tl,
        &ids[0],
        &ids[0],
        "old important idea",
        Some(5),
        vec![],
        None,
    )
    .unwrap();
    let r = db.context_budget(&tl, &ids[4], 100).unwrap();
    assert!(r.messages.iter().any(|m| m.text == "message-0"));
    assert!(!r.messages.iter().any(|m| m.text == "message-3"));
    assert!(r.total_tokens <= 132);
}
#[test]
fn axiom_watermark_integrity() {
    let (mut db, tl, ids) = chain(&[2; 6]);
    db.summarize(&tl, &ids[2], &ids[5], "tail", Some(1), vec![], None)
        .unwrap();
    assert!(db.timeline(&tl).unwrap().watermark.is_none());
    db.summarize(&tl, &ids[0], &ids[5], "all", Some(1), vec![], None)
        .unwrap();
    let t = db.timeline(&tl).unwrap();
    assert_eq!(t.watermark.as_ref(), Some(&ids[5]));
    for id in ids {
        assert!(t.node(&id).unwrap().pinned || t.covers(&id) > 0);
    }
    t.validate().unwrap();
}
#[test]
fn axiom_watermark_monotonic() {
    let (mut db, tl, ids) = chain(&[2; 50]);
    let mut seed = 0x57415445524d4152;
    for i in 0..1000 {
        let to = next(&mut seed) as usize % ids.len();
        let before = db.timeline(&tl).unwrap().watermark.clone();
        db.summarize(
            &tl,
            &ids[0],
            &ids[to],
            &format!("revision-{i}"),
            Some(1),
            vec![],
            None,
        )
        .unwrap();
        let t = db.timeline(&tl).unwrap();
        if let Some(old) = before {
            assert!(
                t.path(t.watermark.as_ref().unwrap())
                    .unwrap()
                    .contains(&old)
            );
        }
        t.validate().unwrap();
    }
}
#[test]
fn axiom_archive_three_conditions() {
    let (mut db, tl, ids) = chain(&[100, 100, 1]);
    db.timeline_mut(&tl).unwrap().fork(&ids[1], "old").unwrap();
    let check = db.timeline(&tl).unwrap().archive_check("old").unwrap();
    assert!(!check.eligible);
    assert_eq!(check.reasons.len(), 1);
    assert!(check.reasons[0].contains("watermark"));
    db.context_budget(&tl, &ids[2], 1).unwrap();
    let check = db.timeline(&tl).unwrap().archive_check("old").unwrap();
    assert_eq!(check.reasons.len(), 1);
    assert!(check.reasons[0].contains("placeholder"));
    db.summarize(&tl, &ids[0], &ids[1], "real", Some(1), vec![], None)
        .unwrap();
    db.timeline_mut(&tl).unwrap().pin(&ids[0]).unwrap();
    let check = db.timeline(&tl).unwrap().archive_check("old").unwrap();
    assert_eq!(check.reasons.len(), 1);
    assert!(check.reasons[0].contains("pin"));
    db.timeline_mut(&tl).unwrap().unpin(&ids[0]).unwrap();
    assert!(
        db.timeline(&tl)
            .unwrap()
            .archive_check("old")
            .unwrap()
            .eligible
    );
    let before = db.timeline(&tl).unwrap().nodes.clone();
    let archived = db.archive_branch(&tl, "old").unwrap();
    assert!(archived.archived.is_empty());
    assert_eq!(db.timeline(&tl).unwrap().nodes, before);
    assert!(!db.timeline(&tl).unwrap().branches.contains_key("old"));
    assert_eq!(db.audit.last().unwrap().op, "archive_branch");

    // v1.1.1: summary coverage no longer prevents exclusive source movement.
    let (mut db, tl, ids) = chain(&[10, 20, 30]);
    db.timeline_mut(&tl).unwrap().fork(&ids[0], "live").unwrap();
    db.timeline_mut(&tl).unwrap().switch("live").unwrap();
    let summary = db
        .summarize(
            &tl,
            &ids[0],
            &ids[2],
            "all original messages",
            Some(3),
            vec![],
            None,
        )
        .unwrap();
    let original = db.timeline(&tl).unwrap().context(&ids[2]).unwrap();
    let moved = db.archive_branch(&tl, "main").unwrap();
    assert_eq!(moved.archived, ids[1..]);
    let t = db.timeline(&tl).unwrap();
    assert!(t.nodes.contains_key(&ids[0]));
    assert!(t.nodes.contains_key(&summary));
    assert_eq!(t.archive.len(), 2);
    assert_eq!(t.context(&ids[2]).unwrap(), original);
    assert_eq!(t.watermark.as_ref(), Some(&ids[2]));
    t.validate().unwrap();
}
#[test]
fn axiom_idempotency_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::new(dir.path().join("timeline.json"));
    let tl = store
        .transaction("create", None, |db| Ok(json!(db.create()?)))
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let first = store
        .transaction(&tl, Some("append"), |db| {
            Ok(json!({"node":db.append(&tl,"user","once",Some(5))?}))
        })
        .unwrap();
    let second = store
        .transaction(&tl, Some("append"), |_| panic!("must not execute twice"))
        .unwrap();
    assert_eq!(first, second);
    let node = first["node"].as_str().unwrap();
    let first = store
        .transaction(&tl, Some("summary"), |db| {
            Ok(json!({"node":db.summarize(&tl,node,node,"once",Some(1),vec![],None)?}))
        })
        .unwrap();
    let count = store.read().unwrap().next_node;
    let second = store
        .transaction(&tl, Some("summary"), |_| panic!("must not execute twice"))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(store.read().unwrap().next_node, count);
}
#[test]
fn axiom_summary_version_chain() {
    let (mut db, tl, ids) = chain(&[5, 6]);
    let a = db
        .summarize(&tl, &ids[0], &ids[0], "A", Some(1), vec![], None)
        .unwrap();
    let b = db
        .summarize(&tl, &ids[1], &ids[1], "B", Some(1), vec![a.clone()], None)
        .unwrap();
    let a2 = db
        .summarize(
            &tl,
            &ids[0],
            &ids[0],
            "A corrected",
            Some(2),
            vec![],
            Some(a.clone()),
        )
        .unwrap();
    let t = db.timeline(&tl).unwrap();
    assert_eq!(t.node(&a2).unwrap().summary_version, 2);
    assert_eq!(t.node(&a2).unwrap().supersedes.as_ref(), Some(&a));
    assert_eq!(t.node(&a).unwrap().message.as_ref().unwrap().text, "A");
    assert!(t.node(&b).unwrap().stale);
}
#[test]
fn storage_roundtrip_and_watermark_toggle_equivalence() {
    assert_eq!(estimate_tokens(""), 1);
    assert_eq!(estimate_tokens("🦀中文ab"), 2);
    let (mut db, tl, ids) = chain(&[50; 10]);
    db.summarize(&tl, &ids[0], &ids[4], "first half", Some(3), vec![], None)
        .unwrap();
    let mut copy: Database = serde_json::from_str(&serde_json::to_string(&db).unwrap()).unwrap();
    copy.rebuild_and_validate().unwrap();
    assert_eq!(
        serde_json::to_value(&db).unwrap(),
        serde_json::to_value(&copy).unwrap()
    );
    let a = db
        .context_budget_with_watermark(&tl, &ids[9], 150, false)
        .unwrap();
    let b = copy
        .context_budget_with_watermark(&tl, &ids[9], 150, true)
        .unwrap();
    assert_eq!(a, b);
}
