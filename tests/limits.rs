use timeline::{Database, MAX_MESSAGES, MAX_TEXT_BYTES, RESERVE, estimate_tokens};

#[test]
fn text_limit_counts_utf8_bytes_and_rejects_without_allocating_an_id() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let text = "a".repeat(MAX_TEXT_BYTES);
    db.append(&tl, "user", &text, None).unwrap();
    let before = db.next_node;
    let text = "🦀".repeat(MAX_TEXT_BYTES / 4 + 1);
    assert!(db.append(&tl, "user", &text, None).is_err());
    assert_eq!(before, db.next_node);
    assert_eq!(estimate_tokens("🦀🦀🦀🦀"), 1);
}

#[test]
fn oversized_context_can_be_budgeted_and_oversized_pinned_output_is_rejected() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let mut ids = Vec::new();
    for _ in 0..=MAX_MESSAGES {
        ids.push(db.append(&tl, "user", "message", Some(1)).unwrap());
    }
    let last = ids.last().unwrap();
    assert!(
        db.timeline(&tl)
            .unwrap()
            .context(last)
            .unwrap_err()
            .to_string()
            .contains("use budget")
    );
    let result = db.context_budget(&tl, last, 1).unwrap();
    assert_eq!(result.messages.len(), 2);
    assert_eq!(result.total_tokens, RESERVE + 1);
    let t = db.timeline_mut(&tl).unwrap();
    for id in &ids {
        t.pin(id).unwrap();
    }
    let count = db.next_node;
    let audits = db.audit.len();
    assert!(
        db.context_budget(&tl, last, 1)
            .unwrap_err()
            .to_string()
            .contains("100000")
    );
    assert_eq!(count, db.next_node);
    assert_eq!(audits, db.audit.len());
}

#[test]
fn reserve_bounds_the_placeholder_estimator_at_maximum_id_width() {
    let text = format!(
        "[自动摘要占位：{} 条消息（n{}→n{}），调用 summarize 写入真实摘要]",
        u64::MAX,
        u64::MAX,
        u64::MAX
    );
    assert!(estimate_tokens(&text) <= RESERVE);
}

#[test]
fn refcount_tracks_branch_heads_summary_coverage_and_pin_once() {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let a = db.append(&tl, "user", "A", Some(2)).unwrap();
    let b = db.append(&tl, "user", "B", Some(3)).unwrap();
    db.timeline_mut(&tl).unwrap().fork(&a, "b").unwrap();
    db.timeline_mut(&tl).unwrap().pin(&a).unwrap();
    let summary = db
        .summarize(&tl, &a, &b, "A and B", Some(1), vec![], None)
        .unwrap();
    assert_eq!(db.timeline(&tl).unwrap().refcount(&a).unwrap(), 4);
    assert_eq!(db.timeline(&tl).unwrap().refcount(&b).unwrap(), 2);
    assert_eq!(db.timeline(&tl).unwrap().refcount(&summary).unwrap(), 0);
    db.timeline_mut(&tl).unwrap().drop_branch("b").unwrap();
    db.timeline_mut(&tl).unwrap().unpin(&a).unwrap();
    assert_eq!(db.timeline(&tl).unwrap().refcount(&a).unwrap(), 2);
    let mut copy: Database = serde_json::from_value(serde_json::to_value(&db).unwrap()).unwrap();
    copy.rebuild_and_validate().unwrap();
    assert_eq!(copy.timeline(&tl).unwrap().refcount(&a).unwrap(), 2);
}
