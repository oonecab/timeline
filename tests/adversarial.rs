//! Adversarial checks for source preservation, derived reconstruction, and
//! interacting operations. These supplement, rather than replace, §9 axioms.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use timeline::{Database, Timeline};

fn chain(costs: &[u64]) -> (Database, String, Vec<String>) {
    let mut db = Database::default();
    let tl = db.create().unwrap();
    let ids = costs
        .iter()
        .enumerate()
        .map(|(i, &cost)| {
            db.append(&tl, "user", &format!("original-{i}"), Some(cost))
                .unwrap()
        })
        .collect();
    (db, tl, ids)
}

fn ancestry(tl: &Timeline, node: &str) -> Vec<String> {
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursor = Some(node.to_owned());
    while let Some(id) = cursor {
        assert!(seen.insert(id.clone()), "parent cycle at {id}");
        let n = tl.node(&id).unwrap();
        path.push(id);
        cursor = n.parent.clone();
    }
    path.reverse();
    path
}

fn covered(tl: &Timeline, node: &str) -> bool {
    tl.nodes.values().any(|summary| {
        summary.summary_range.as_ref().is_some_and(|range| {
            let path = ancestry(tl, &range.to);
            let start = path.iter().position(|id| id == &range.from).unwrap();
            path[start..].iter().any(|id| id == node)
        })
    })
}

fn immutable_sources(tl: &Timeline) -> BTreeMap<String, Value> {
    tl.nodes
        .iter()
        .map(|(id, node)| {
            let mut value = serde_json::to_value(node).unwrap();
            let object = value.as_object_mut().unwrap();
            // Only pinned/children and the explicitly derived stale flag may
            // change during valid operations. prefix_tokens remains fixed.
            object.remove("pinned");
            object.remove("children");
            object.remove("stale");
            (id.clone(), value)
        })
        .collect()
}

fn assert_independent_invariants(tl: &Timeline) {
    let roots = tl.nodes.values().filter(|n| n.parent.is_none()).count();
    assert_eq!(roots, 1);
    assert!(tl.branches.contains_key(&tl.current_branch));
    for head in tl.branches.values() {
        assert!(tl.nodes.contains_key(head));
    }
    let mut successors = BTreeSet::new();
    for (id, node) in &tl.nodes {
        assert_eq!(id, &node.id);
        let path = ancestry(tl, id);
        let expected: u64 = path
            .iter()
            .filter_map(|p| tl.nodes[p].message.as_ref())
            .map(|m| m.tokens)
            .sum();
        assert_eq!(node.prefix_tokens, expected);
        assert_eq!(tl.tokens(id).unwrap(), expected);
        if let Some(parent) = &node.parent {
            assert_eq!(
                tl.nodes[parent]
                    .children
                    .iter()
                    .filter(|c| *c == id)
                    .count(),
                1
            );
        }
        let children: BTreeSet<_> = node.children.iter().collect();
        assert_eq!(children.len(), node.children.len());
        for child in &node.children {
            assert_eq!(tl.nodes[child].parent.as_deref(), Some(id.as_str()));
        }
        if let Some(range) = &node.summary_range {
            assert!(ancestry(tl, &range.to).contains(&range.from));
            assert_eq!(node.parent, tl.nodes[&range.from].parent);
            assert_eq!(node.message.as_ref().unwrap().role, "summary");
        }
        if let Some(previous) = &node.supersedes {
            assert!(successors.insert(previous), "summary version chain forks");
            let previous = &tl.nodes[previous];
            assert_eq!(node.summary_range, previous.summary_range);
            assert_eq!(node.summary_version, previous.summary_version + 1);
        }
        for dependency in &node.derived_from {
            assert!(tl.nodes[dependency].summary_range.is_some());
        }
    }
    if let Some(watermark) = &tl.watermark {
        for id in ancestry(tl, watermark).iter().skip(1) {
            assert!(tl.nodes[id].pinned || covered(tl, id), "bare node {id}");
        }
    }
    tl.validate().unwrap();
}

#[test]
fn unpin_cannot_invalidate_an_established_watermark() {
    let (mut db, tl, ids) = chain(&[4, 5, 6]);
    db.timeline_mut(&tl).unwrap().pin(&ids[0]).unwrap();
    db.summarize(&tl, &ids[1], &ids[2], "tail summary", Some(2), vec![], None)
        .unwrap();
    assert_eq!(db.timeline(&tl).unwrap().watermark.as_ref(), Some(&ids[2]));
    let before = serde_json::to_value(&db).unwrap();
    assert!(db.timeline_mut(&tl).unwrap().unpin(&ids[0]).is_err());
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    db.summarize(
        &tl,
        &ids[0],
        &ids[0],
        "covered first",
        Some(1),
        vec![],
        None,
    )
    .unwrap();
    db.timeline_mut(&tl).unwrap().unpin(&ids[0]).unwrap();
    assert_independent_invariants(db.timeline(&tl).unwrap());
}

#[test]
fn real_summary_preserves_placeholder_and_clears_archive_dependency() {
    let (mut db, tl, ids) = chain(&[100, 100, 1]);
    db.timeline_mut(&tl).unwrap().fork(&ids[1], "old").unwrap();
    let budget = db.context_budget(&tl, &ids[2], 1).unwrap();
    assert_eq!(budget.summaries_created.len(), 1);
    let placeholder = budget.summaries_created[0].clone();
    let before =
        serde_json::to_value(db.timeline(&tl).unwrap().node(&placeholder).unwrap()).unwrap();
    let summary = db
        .summarize(&tl, &ids[0], &ids[1], "real summary", Some(2), vec![], None)
        .unwrap();
    assert_ne!(summary, placeholder);
    assert_eq!(
        serde_json::to_value(db.timeline(&tl).unwrap().node(&placeholder).unwrap()).unwrap(),
        before
    );
    let check = db.timeline(&tl).unwrap().archive_check("old").unwrap();
    assert!(check.eligible, "{:?}", check.reasons);
    let moved = db.archive_branch(&tl, "old").unwrap();
    // This entire path is shared with main, so even the v1.1.1 relaxed
    // archive rule must leave the shared original nodes hot.
    assert!(moved.archived.is_empty());
    for id in &ids {
        assert!(db.timeline(&tl).unwrap().nodes.contains_key(id));
    }
    assert_independent_invariants(db.timeline(&tl).unwrap());
}

#[test]
fn summary_revision_staleness_cascades_without_rewriting_history() {
    let (mut db, tl, ids) = chain(&[2, 3, 4]);
    let a = db
        .summarize(&tl, &ids[0], &ids[0], "A", Some(1), vec![], None)
        .unwrap();
    let b = db
        .summarize(&tl, &ids[1], &ids[1], "B", Some(1), vec![a.clone()], None)
        .unwrap();
    let c = db
        .summarize(&tl, &ids[2], &ids[2], "C", Some(1), vec![b.clone()], None)
        .unwrap();
    let old = immutable_sources(db.timeline(&tl).unwrap());
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
    let timeline = db.timeline(&tl).unwrap();
    assert_eq!(timeline.node(&a2).unwrap().summary_version, 2);
    assert_eq!(timeline.node(&a2).unwrap().supersedes.as_ref(), Some(&a));
    assert!(timeline.node(&b).unwrap().stale);
    assert!(timeline.node(&c).unwrap().stale);
    for (id, value) in old {
        assert_eq!(immutable_sources(timeline)[&id], value);
    }
    let before = serde_json::to_value(&db).unwrap();
    let repeated = db
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
    assert_eq!(repeated, a2);
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(
        db.summarize(
            &tl,
            &ids[0],
            &ids[0],
            "forked version",
            Some(2),
            vec![],
            Some(a)
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert_independent_invariants(db.timeline(&tl).unwrap());
}

#[test]
fn derived_corruption_is_rebuilt_but_source_corruption_is_rejected() {
    let (mut db, tl, ids) = chain(&[2, 3, 4]);
    let a = db
        .summarize(&tl, &ids[0], &ids[0], "A", Some(1), vec![], None)
        .unwrap();
    let b = db
        .summarize(&tl, &ids[1], &ids[1], "B", Some(1), vec![a.clone()], None)
        .unwrap();
    db.summarize(
        &tl,
        &ids[0],
        &ids[0],
        "A corrected",
        Some(2),
        vec![],
        Some(a),
    )
    .unwrap();
    let clean = serde_json::to_value(&db).unwrap();
    let mut dirty = clean.clone();
    for node in dirty["timelines"][&tl]["nodes"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        node["prefix_tokens"] = json!(999_999);
        node["stale"] = json!(false);
    }
    let mut restored: Database = serde_json::from_value(dirty).unwrap();
    restored.rebuild_and_validate().unwrap();
    assert!(restored.timeline(&tl).unwrap().node(&b).unwrap().stale);
    assert_independent_invariants(restored.timeline(&tl).unwrap());
    assert_eq!(serde_json::to_value(&restored).unwrap(), clean);

    let mut bad_parent = clean.clone();
    bad_parent["timelines"][&tl]["nodes"][&ids[1]]["parent"] = json!("n999999");
    let mut restored: Database = serde_json::from_value(bad_parent).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut missing_child = clean.clone();
    missing_child["timelines"][&tl]["nodes"][&ids[0]]["children"] = json!([]);
    let mut restored: Database = serde_json::from_value(missing_child).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut bad_dependency = clean;
    bad_dependency["timelines"][&tl]["nodes"][&b]["derived_from"] = json!([ids[0]]);
    let mut restored: Database = serde_json::from_value(bad_dependency).unwrap();
    assert!(restored.rebuild_and_validate().is_err());
}

#[test]
fn malformed_derived_json_values_are_discarded_before_reconstruction() {
    let (db, tl, ids) = chain(&[2, 3, 4]);
    let clean = serde_json::to_value(&db).unwrap();
    let mut malformed = clean.clone();
    malformed["timelines"][&tl]["nodes"][&ids[0]]["prefix_tokens"] = json!("broken cache");
    malformed["timelines"][&tl]["nodes"][&ids[1]]["stale"] = Value::Null;
    let mut rebuilt: Database = serde_json::from_value(malformed)
        .expect("malformed derived caches are not malformed source data (§6.3)");
    rebuilt.rebuild_and_validate().unwrap();
    assert_eq!(serde_json::to_value(rebuilt).unwrap(), clean);
}

#[test]
fn cyclic_or_nonmonotonic_source_metadata_is_rejected() {
    let (mut db, tl, ids) = chain(&[2, 3, 4]);
    let summary = db
        .summarize(&tl, &ids[0], &ids[1], "first two", Some(1), vec![], None)
        .unwrap();
    let clean = serde_json::to_value(&db).unwrap();

    let mut cyclic_parent = clean.clone();
    cyclic_parent["timelines"][&tl]["nodes"][&ids[0]]["parent"] = json!(ids[1]);
    let mut restored: Database = serde_json::from_value(cyclic_parent).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut cyclic_dependency = clean.clone();
    cyclic_dependency["timelines"][&tl]["nodes"][&summary]["derived_from"] = json!([summary]);
    let mut restored: Database = serde_json::from_value(cyclic_dependency).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut invalid_watermark = clean.clone();
    invalid_watermark["timelines"][&tl]["watermark"] = json!(ids[2]);
    let mut restored: Database = serde_json::from_value(invalid_watermark).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut stale_counter = clean.clone();
    stale_counter["next_node"] = json!(0);
    let mut restored: Database = serde_json::from_value(stale_counter).unwrap();
    assert!(restored.rebuild_and_validate().is_err());

    let mut stale_counter = clean;
    stale_counter["next_timeline"] = json!(0);
    let mut restored: Database = serde_json::from_value(stale_counter).unwrap();
    assert!(restored.rebuild_and_validate().is_err());
}

#[test]
fn archived_ancestor_is_readable_but_cannot_be_an_active_branch_head() {
    let (db, tl, ids) = chain(&[1, 1]);
    let mut malformed = serde_json::to_value(&db).unwrap();
    let ancestor = malformed["timelines"][&tl]["nodes"]
        .as_object_mut()
        .unwrap()
        .remove(&ids[0])
        .unwrap();
    malformed["timelines"][&tl]["archive"][&ids[0]] = ancestor;
    let mut rebuilt: Database = serde_json::from_value(malformed.clone()).unwrap();
    rebuilt.rebuild_and_validate().unwrap();
    assert_eq!(
        rebuilt.timeline(&tl).unwrap().context(&ids[1]).unwrap(),
        db.timeline(&tl).unwrap().context(&ids[1]).unwrap()
    );
    assert_eq!(rebuilt.timeline(&tl).unwrap().tokens(&ids[1]).unwrap(), 2);
    rebuilt.timeline(&tl).unwrap().validate().unwrap();

    malformed["timelines"][&tl]["branches"]["main"] = json!(ids[0]);
    let mut rebuilt: Database = serde_json::from_value(malformed).unwrap();
    assert!(rebuilt.rebuild_and_validate().is_err());
}

#[test]
fn archived_nodes_still_require_valid_summary_source_metadata() {
    let (mut db, tl, ids) = chain(&[2, 3, 4]);
    // Leave the initial message uncovered so no watermark depends on this
    // summary. The rejection must come from malformed archive metadata itself.
    let summary = db
        .summarize(&tl, &ids[1], &ids[1], "middle only", Some(1), vec![], None)
        .unwrap();
    assert!(db.timeline(&tl).unwrap().watermark.is_none());
    let mut malformed = serde_json::to_value(&db).unwrap();
    let mut summary_node = malformed["timelines"][&tl]["nodes"]
        .as_object_mut()
        .unwrap()
        .remove(&summary)
        .unwrap();
    summary_node["summary_range"] = json!({"from":"n999999","to":"n999999"});
    malformed["timelines"][&tl]["archive"][&summary] = summary_node;
    let mut rebuilt: Database = serde_json::from_value(malformed).unwrap();
    assert!(rebuilt.rebuild_and_validate().is_err());

    let mut malformed = serde_json::to_value(&db).unwrap();
    let mut ordinary = malformed["timelines"][&tl]["nodes"]
        .as_object_mut()
        .unwrap()
        .remove(&ids[2])
        .unwrap();
    ordinary["derived_from"] = json!(["n999999"]);
    malformed["timelines"][&tl]["archive"][&ids[2]] = ordinary;
    malformed["timelines"][&tl]["branches"]["main"] = json!(ids[1]);
    let mut rebuilt: Database = serde_json::from_value(malformed).unwrap();
    assert!(rebuilt.rebuild_and_validate().is_err());
}

#[test]
fn archived_summary_caches_and_active_downstream_staleness_are_rebuilt() {
    let (mut db, tl, ids) = chain(&[2, 3, 4]);
    let a = db
        .summarize(&tl, &ids[0], &ids[0], "A", Some(1), vec![], None)
        .unwrap();
    let b = db
        .summarize(&tl, &ids[1], &ids[1], "B", Some(1), vec![a.clone()], None)
        .unwrap();
    let c = db
        .summarize(&tl, &ids[2], &ids[2], "C", Some(1), vec![b.clone()], None)
        .unwrap();
    db.summarize(&tl, &ids[0], &ids[0], "A revised", Some(1), vec![], Some(a))
        .unwrap();
    // A saved archive may contain summary nodes from earlier producers. The
    // format must still validate and rebuild them even though archive_branch
    // itself preserves independent summary nodes in the hot area.
    let timeline = db.timeline_mut(&tl).unwrap();
    let archived = timeline.nodes.remove(&b).unwrap();
    timeline.archive.insert(b.clone(), archived);
    db.rebuild_and_validate().unwrap();
    let clean = serde_json::to_value(&db).unwrap();
    let mut dirty = clean.clone();
    dirty["timelines"][&tl]["archive"][&b]["prefix_tokens"] = json!("broken cache");
    dirty["timelines"][&tl]["archive"][&b]["stale"] = json!(false);
    dirty["timelines"][&tl]["nodes"][&c]["stale"] = json!(false);
    let mut restored: Database = serde_json::from_value(dirty).unwrap();
    restored.rebuild_and_validate().unwrap();
    let timeline = restored.timeline(&tl).unwrap();
    assert!(timeline.archive[&b].stale);
    assert!(timeline.node(&c).unwrap().stale);
    timeline.validate().unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), clean);
}

#[test]
fn root_is_empty_context_and_cannot_be_replaced_by_a_summary() {
    let (mut db, tl, ids) = chain(&[2, 3]);
    let root = ancestry(db.timeline(&tl).unwrap(), &ids[0])[0].clone();
    assert!(db.timeline(&tl).unwrap().context(&root).unwrap().is_empty());
    assert_eq!(db.timeline(&tl).unwrap().tokens(&root).unwrap(), 0);
    let before = serde_json::to_value(&db).unwrap();
    let result = db.context_budget(&tl, &root, 0).unwrap();
    assert!(result.messages.is_empty());
    assert!(result.summaries_created.is_empty());
    assert_eq!(result.total_tokens, 0);
    assert!(
        db.summarize(
            &tl,
            &root,
            &ids[1],
            "invalid root summary",
            Some(1),
            vec![],
            None
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn arithmetic_overflow_is_explicit_and_atomic() {
    let (mut db, tl, ids) = chain(&[u64::MAX - 1, 1]);
    let before = serde_json::to_value(&db).unwrap();
    assert!(db.append(&tl, "user", "overflow", Some(1)).is_err());
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(
        db.summarize(
            &tl,
            &ids[1],
            &ids[1],
            "overflow summary",
            Some(2),
            vec![],
            None
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    let context = db.context_budget(&tl, &ids[1], u64::MAX).unwrap();
    assert_eq!(context.total_tokens, u64::MAX);
    assert_eq!(context.messages.len(), 2);
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn rejected_mutations_do_not_consume_ids_or_modify_source_state() {
    let (mut db, tl, ids) = chain(&[2, 3]);
    let before = serde_json::to_value(&db).unwrap();
    assert!(db.append(&tl, "unknown-role", "bad", Some(1)).is_err());
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(
        db.summarize(&tl, &ids[1], &ids[0], "backwards", Some(1), vec![], None)
            .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(
        db.summarize(
            &tl,
            &ids[0],
            &ids[1],
            "bad dependency",
            Some(1),
            vec![ids[0].clone()],
            None
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(
        db.timeline_mut(&tl)
            .unwrap()
            .fork(&ids[0], "bad/name")
            .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    assert!(db.timeline_mut(&tl).unwrap().drop_branch("main").is_err());
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn oversized_target_reports_infeasibility_without_mutation() {
    // §5.2 requires keeping the target; §5.3 cannot bound 100 by 0 + 32.
    let (mut db, tl, ids) = chain(&[100]);
    let before = serde_json::to_value(&db).unwrap();
    let error = db.context_budget(&tl, &ids[0], 0).unwrap_err();
    assert!(error.to_string().contains("budget infeasible"), "{error}");
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn separated_mandatory_nodes_report_infeasibility_without_mutation() {
    // The pinned message itself does NOT exceed the budget. Keeping it forces
    // two distinct omitted intervals: 32 + 1 + 32 + 0 > max(1) + 32.
    let (mut db, tl, ids) = chain(&[1000, 1, 1000, 0]);
    db.timeline_mut(&tl).unwrap().pin(&ids[1]).unwrap();
    let before = serde_json::to_value(&db).unwrap();
    let error = db.context_budget(&tl, &ids[3], 1).unwrap_err();
    assert!(error.to_string().contains("budget infeasible"), "{error}");
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn optional_covered_nodes_do_not_force_multiple_gap_budget_overflow() {
    // The literal Tier 2 pass would keep original-1 and emit two placeholders
    // for a total of 65. A legal layout drops the entire optional interval for
    // one placeholder and retains the target: 32 <= max(1) + 32.
    let (mut db, tl, ids) = chain(&[1000, 1, 1000, 0]);
    db.summarize(&tl, &ids[1], &ids[1], "semantic hub", Some(1), vec![], None)
        .unwrap();
    let before = immutable_sources(db.timeline(&tl).unwrap());
    let result = db.context_budget(&tl, &ids[3], 1).unwrap();
    assert!(result.total_tokens <= 33);
    assert_eq!(result.messages.last().unwrap().text, "original-3");
    assert_eq!(result.summaries_created.len(), 1);
    let summary = db
        .timeline(&tl)
        .unwrap()
        .node(&result.summaries_created[0])
        .unwrap();
    let range = summary.summary_range.as_ref().unwrap();
    assert_eq!(range.from, ids[0]);
    assert_eq!(range.to, ids[2]);
    let after = immutable_sources(db.timeline(&tl).unwrap());
    for (id, value) in before {
        assert_eq!(after[&id], value);
    }
    assert_independent_invariants(db.timeline(&tl).unwrap());
}

#[test]
fn placeholder_coverage_does_not_change_tiers_on_a_repeat_request() {
    let (mut db, tl, ids) = chain(&[50; 10]);
    let first = db.context_budget(&tl, &ids[9], 150).unwrap();
    let before = serde_json::to_value(&db).unwrap();
    let second = db.context_budget(&tl, &ids[9], 150).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn placeholder_cannot_be_a_version_predecessor_and_ranges_must_match() {
    let (mut db, tl, ids) = chain(&[100, 100, 1]);
    let budget = db.context_budget(&tl, &ids[2], 1).unwrap();
    let placeholder = budget.summaries_created[0].clone();
    let before = serde_json::to_value(&db).unwrap();
    assert!(
        db.summarize(
            &tl,
            &ids[0],
            &ids[1],
            "real",
            Some(1),
            vec![],
            Some(placeholder)
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
    let real = db
        .summarize(&tl, &ids[0], &ids[1], "real", Some(1), vec![], None)
        .unwrap();
    let before = serde_json::to_value(&db).unwrap();
    assert!(
        db.summarize(
            &tl,
            &ids[1],
            &ids[2],
            "wrong range",
            Some(1),
            vec![],
            Some(real)
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&db).unwrap(), before);
}

#[test]
fn multiple_placeholder_creation_advances_to_the_last_complete_prefix() {
    // Two omitted intervals surround an unpinned, uncovered kept message.
    // The first new placeholder establishes a valid watermark; attempting to
    // advance only to the final placeholder would lose that successful advance.
    let (mut db, tl, ids) = chain(&[1000, 1, 1000, 1]);
    let result = db.context_budget(&tl, &ids[3], 50).unwrap();
    assert_eq!(result.summaries_created.len(), 2);
    assert_eq!(db.timeline(&tl).unwrap().watermark.as_ref(), Some(&ids[0]));
    assert_independent_invariants(db.timeline(&tl).unwrap());
}

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick(&mut self, len: usize) -> usize {
        self.next() as usize % len
    }
}

#[test]
fn one_thousand_budgets_match_independent_feasibility_and_required_messages() {
    let mut rng = Rng(0xb0_d6_e7_20_26);
    for case in 0..1000 {
        let count = 1 + rng.pick(18);
        let costs: Vec<u64> = (0..count).map(|_| rng.next() % 81).collect();
        let (mut db, tl, ids) = chain(&costs);
        let mut pins = vec![false; count];
        for (index, id) in ids.iter().enumerate() {
            if rng.next() % 5 == 0 {
                pins[index] = true;
                db.timeline_mut(&tl).unwrap().pin(id).unwrap();
            }
            if rng.next() % 6 == 0 {
                db.summarize(&tl, id, id, &format!("hub-{index}"), Some(1), vec![], None)
                    .unwrap();
            }
        }
        let budget = rng.next() % 201;
        let pin_total: u64 = pins
            .iter()
            .zip(&costs)
            .filter_map(|(pinned, cost)| pinned.then_some(*cost))
            .sum();
        // Independent lower bound: every target/pin must survive. Each gap
        // between them costs either all original tokens or a single 32-token
        // placeholder, whichever is less. This proves which cases are feasible.
        let mut minimum = 0;
        let mut gap_cost = 0;
        for index in 0..count {
            if pins[index] || index == count - 1 {
                minimum += gap_cost.min(32) + costs[index];
                gap_cost = 0;
            } else {
                gap_cost += costs[index];
            }
        }
        let before = serde_json::to_value(&db).unwrap();
        let result = db.context_budget(&tl, &ids[count - 1], budget);
        if pin_total <= budget && minimum > budget + 32 {
            let error = result.expect_err("mathematically infeasible context must be explicit");
            assert!(error.to_string().contains("budget infeasible"));
            assert_eq!(serde_json::to_value(&db).unwrap(), before);
            continue;
        }
        let result = result.unwrap_or_else(|error| panic!("feasible case {case}: {error}"));
        let sum: u64 = result.messages.iter().map(|message| message.tokens).sum();
        assert_eq!(result.total_tokens, sum);
        if pin_total <= budget {
            assert!(sum <= budget + 32, "case {case}: {sum} > {budget} + 32");
        }
        for (index, pinned) in pins.iter().enumerate() {
            if *pinned || index == count - 1 {
                assert!(
                    result
                        .messages
                        .iter()
                        .any(|m| m.text == format!("original-{index}"))
                );
            }
        }
        let unique: BTreeSet<_> = result.messages.iter().map(|m| &m.text).collect();
        assert_eq!(unique.len(), result.messages.len());
        assert_independent_invariants(db.timeline(&tl).unwrap());
        let after_first = serde_json::to_value(&db).unwrap();
        let repeated = db.context_budget(&tl, &ids[count - 1], budget).unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::to_value(repeated).unwrap()
        );
        assert_eq!(serde_json::to_value(&db).unwrap(), after_first);
    }
}

#[test]
fn seeded_mixed_operations_preserve_all_source_fields_and_watermark() {
    let mut rng = Rng(0x5e_ed_10_13);
    for case in 0..20 {
        let (mut db, tl, _) = chain(&[3, 5, 7]);
        for step in 0..60 {
            let timeline = db.timeline(&tl).unwrap();
            let original = immutable_sources(timeline);
            let watermark = timeline.watermark.clone();
            let mut ids: Vec<String> = timeline.nodes.keys().cloned().collect();
            ids.sort();
            let chosen = ids[rng.pick(ids.len())].clone();
            let head = timeline.branches[&timeline.current_branch].clone();
            let path = ancestry(timeline, &head);
            match rng.next() % 9 {
                0 => {
                    db.append(
                        &tl,
                        "assistant",
                        &format!("case-{case}-step-{step}"),
                        Some(rng.next() % 120),
                    )
                    .unwrap();
                }
                1 => {
                    db.timeline_mut(&tl)
                        .unwrap()
                        .fork(&chosen, &format!("b{step}"))
                        .unwrap();
                    assert_eq!(db.timeline(&tl).unwrap().nodes.len(), original.len());
                }
                2 => {
                    let mut names: Vec<_> = timeline.branches.keys().cloned().collect();
                    names.sort();
                    db.timeline_mut(&tl)
                        .unwrap()
                        .switch(&names[rng.pick(names.len())])
                        .unwrap();
                }
                3 => db.timeline_mut(&tl).unwrap().pin(&chosen).unwrap(),
                4 => {
                    let protected = timeline
                        .watermark
                        .as_ref()
                        .is_some_and(|w| ancestry(timeline, w).contains(&chosen))
                        && timeline.node(&chosen).unwrap().message.is_some()
                        && !covered(timeline, &chosen);
                    let result = db.timeline_mut(&tl).unwrap().unpin(&chosen);
                    assert_eq!(result.is_err(), protected);
                }
                5 if path.len() > 1 => {
                    let a = 1 + rng.pick(path.len() - 1);
                    let b = a + rng.pick(path.len() - a);
                    db.summarize(
                        &tl,
                        &path[a],
                        &path[b],
                        &format!("summary-{case}-{step}"),
                        Some(1),
                        vec![],
                        None,
                    )
                    .unwrap();
                }
                6 => {
                    let mut candidates: Vec<_> = timeline
                        .branches
                        .keys()
                        .filter(|name| *name != &timeline.current_branch)
                        .cloned()
                        .collect();
                    candidates.sort();
                    if !candidates.is_empty() {
                        db.timeline_mut(&tl)
                            .unwrap()
                            .drop_branch(&candidates[rng.pick(candidates.len())])
                            .unwrap();
                    }
                }
                7 => {
                    let expected = timeline.context(&head).unwrap();
                    let enough = timeline.tokens(&head).unwrap();
                    let result = db.context_budget(&tl, &head, enough).unwrap();
                    assert_eq!(
                        serde_json::to_value(result.messages).unwrap(),
                        serde_json::to_value(expected).unwrap()
                    );
                    assert_eq!(result.total_tokens, enough);
                }
                _ => {
                    let before = serde_json::to_value(&db).unwrap();
                    assert!(db.append(&tl, "invalid", "no-op", Some(1)).is_err());
                    assert_eq!(serde_json::to_value(&db).unwrap(), before);
                }
            }
            let timeline = db.timeline(&tl).unwrap();
            let after = immutable_sources(timeline);
            for (id, value) in original {
                assert_eq!(
                    after[&id], value,
                    "source changed in case {case} step {step}"
                );
            }
            if let Some(previous) = watermark {
                let current = timeline.watermark.as_ref().expect("watermark disappeared");
                assert!(ancestry(timeline, current).contains(&previous));
            }
            assert_independent_invariants(timeline);
        }
    }
}
