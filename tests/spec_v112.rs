//! Regression coverage for the v1.1.1 archive and v1.1.2 budget contracts.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use timeline::storage::Storage;
use timeline::{Database, TLNode, Timeline};

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

struct ArchiveFixture {
    db: Database,
    tl: String,
    originals: Vec<String>,
    live: String,
    summary: String,
    dependent: String,
    downstream: String,
}

fn archive_fixture() -> ArchiveFixture {
    let (mut db, tl, originals) = chain(&[3, 5, 7, 11]);
    db.timeline_mut(&tl)
        .unwrap()
        .fork(&originals[0], "live")
        .unwrap();
    db.timeline_mut(&tl).unwrap().switch("live").unwrap();
    let live = db
        .append(&tl, "assistant", "live branch", Some(13))
        .unwrap();
    db.summarize(
        &tl,
        &originals[0],
        &originals[0],
        "shared prefix",
        Some(1),
        vec![],
        None,
    )
    .unwrap();
    let summary = db
        .summarize(
            &tl,
            &originals[1],
            &originals[3],
            "archivable tail",
            Some(2),
            vec![],
            None,
        )
        .unwrap();
    // Their parents will move to archive while these independent summaries stay hot.
    let dependent = db
        .summarize(
            &tl,
            &originals[2],
            &originals[2],
            "depends on tail",
            Some(1),
            vec![summary.clone()],
            None,
        )
        .unwrap();
    let downstream = db
        .summarize(
            &tl,
            &originals[3],
            &originals[3],
            "depends on dependent",
            Some(1),
            vec![dependent.clone()],
            None,
        )
        .unwrap();
    ArchiveFixture {
        db,
        tl,
        originals,
        live,
        summary,
        dependent,
        downstream,
    }
}

fn all_nodes(t: &Timeline) -> BTreeMap<String, TLNode> {
    t.nodes
        .iter()
        .chain(t.archive.iter())
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect()
}

// Independently inspect the combined graph, rather than assuming every parent is hot.
fn assert_combined_invariants(t: &Timeline) {
    let nodes = all_nodes(t);
    assert_eq!(nodes.len(), t.nodes.len() + t.archive.len());
    assert_eq!(nodes.values().filter(|n| n.parent.is_none()).count(), 1);
    assert!(t.branches.contains_key(&t.current_branch));
    for head in t.branches.values() {
        assert!(t.nodes.contains_key(head), "branch heads must remain hot");
    }
    let mut successors = BTreeSet::new();
    for (id, node) in &nodes {
        assert_eq!(id, &node.id);
        let path = t.path(id).unwrap();
        assert_eq!(path.iter().collect::<BTreeSet<_>>().len(), path.len());
        let cost: u64 = path
            .iter()
            .filter_map(|p| nodes[p].message.as_ref())
            .map(|m| m.tokens)
            .sum();
        assert_eq!(node.prefix_tokens, cost);
        assert_eq!(t.tokens(id).unwrap(), cost);
        if let Some(parent) = &node.parent {
            assert_eq!(
                nodes[parent].children.iter().filter(|c| *c == id).count(),
                1
            );
        } else {
            assert!(t.nodes.contains_key(id));
        }
        assert_eq!(
            node.children.iter().collect::<BTreeSet<_>>().len(),
            node.children.len()
        );
        for child in &node.children {
            assert_eq!(nodes[child].parent.as_deref(), Some(id.as_str()));
        }
        if let Some(range) = &node.summary_range {
            assert!(t.path(&range.to).unwrap().contains(&range.from));
            assert_eq!(node.parent, nodes[&range.from].parent);
        }
        if let Some(previous) = &node.supersedes {
            assert!(successors.insert(previous));
            assert_eq!(node.summary_range, nodes[previous].summary_range);
            assert_eq!(node.summary_version, nodes[previous].summary_version + 1);
        }
        for dependency in &node.derived_from {
            assert!(nodes[dependency].summary_range.is_some());
        }
        let covered_by = nodes
            .values()
            .filter(|summary| {
                summary.summary_range.as_ref().is_some_and(|range| {
                    let path = t.path(&range.to).unwrap();
                    let from = path.iter().position(|p| p == &range.from).unwrap();
                    path[from..].contains(id)
                })
            })
            .count();
        assert_eq!(t.covers(id), covered_by);
        let branches = t
            .branches
            .values()
            .filter(|head| t.path(head).unwrap().contains(id))
            .count();
        assert_eq!(
            t.refcount(id).unwrap(),
            covered_by + branches + usize::from(node.pinned)
        );
    }
    if let Some(watermark) = &t.watermark {
        for id in t.path(watermark).unwrap().iter().skip(1) {
            assert!(nodes[id].pinned || t.covers(id) > 0);
        }
    }
    t.validate().unwrap();
}

#[test]
fn archive_moves_covered_exclusive_originals_and_preserves_all_source_nodes() {
    let mut f = archive_fixture();
    let before = all_nodes(f.db.timeline(&f.tl).unwrap());
    let original_context =
        f.db.timeline(&f.tl)
            .unwrap()
            .context(&f.originals[3])
            .unwrap();
    let live_context = f.db.timeline(&f.tl).unwrap().context(&f.live).unwrap();
    assert!(
        f.db.timeline(&f.tl)
            .unwrap()
            .archive_check("main")
            .unwrap()
            .eligible
    );
    let result = f.db.archive_branch(&f.tl, "main").unwrap();
    assert_eq!(result.archived, f.originals[1..]);
    let t = f.db.timeline(&f.tl).unwrap();
    assert_eq!(
        all_nodes(t),
        before,
        "archive is movement, never a source rewrite"
    );
    assert_eq!(t.archive.len(), 3);
    assert!(t.nodes.contains_key(&f.originals[0]));
    assert!(t.nodes.contains_key(&f.live));
    assert!(t.nodes.contains_key(&f.summary));
    assert!(t.nodes.contains_key(&f.dependent));
    assert!(t.nodes.contains_key(&f.downstream));
    assert_eq!(t.nodes[&f.dependent].parent.as_ref(), Some(&f.originals[1]));
    assert!(t.archive.contains_key(&f.originals[1]));
    assert!(!t.branches.contains_key("main"));
    assert_eq!(t.current_branch, "live");
    assert_eq!(t.watermark.as_ref(), Some(&f.originals[3]));
    assert_eq!(t.context(&f.originals[3]).unwrap(), original_context);
    assert_eq!(t.context(&f.live).unwrap(), live_context);
    let audit = f.db.audit.last().unwrap();
    assert_eq!(audit.op, "archive_branch");
    assert_eq!(
        audit.details,
        json!({"branch":"main", "archived":f.originals[1..]})
    );
    assert_combined_invariants(t);
}

#[test]
fn archive_keeps_shared_originals_until_the_last_branch_reference() {
    let mut f = archive_fixture();
    f.db.timeline_mut(&f.tl)
        .unwrap()
        .fork(&f.originals[2], "shared")
        .unwrap();
    let first = f.db.archive_branch(&f.tl, "main").unwrap();
    assert_eq!(first.archived, vec![f.originals[3].clone()]);
    let t = f.db.timeline(&f.tl).unwrap();
    assert!(t.nodes.contains_key(&f.originals[1]));
    assert!(t.nodes.contains_key(&f.originals[2]));
    assert!(t.archive_check("shared").unwrap().eligible);
    assert_combined_invariants(t);
    let second = f.db.archive_branch(&f.tl, "shared").unwrap();
    assert_eq!(second.archived, f.originals[1..3]);
    assert_combined_invariants(f.db.timeline(&f.tl).unwrap());
}

#[test]
fn archive_keeps_summary_nodes_even_when_they_are_exclusive_branch_ancestors() {
    let (mut db, tl, ids) = chain(&[3, 5, 7]);
    let summary = db
        .summarize(&tl, &ids[2], &ids[2], "replace last", Some(1), vec![], None)
        .unwrap();
    assert!(db.timeline(&tl).unwrap().watermark.is_none());
    db.timeline_mut(&tl)
        .unwrap()
        .fork(&summary, "summary-path")
        .unwrap();
    db.timeline_mut(&tl)
        .unwrap()
        .switch("summary-path")
        .unwrap();
    let exclusive = db
        .append(&tl, "assistant", "summary descendant", Some(2))
        .unwrap();
    db.summarize(
        &tl,
        &ids[0],
        &exclusive,
        "covered summary branch",
        Some(2),
        vec![],
        None,
    )
    .unwrap();
    db.timeline_mut(&tl).unwrap().switch("main").unwrap();
    assert_eq!(
        db.timeline(&tl).unwrap().watermark.as_ref(),
        Some(&exclusive)
    );
    let result = db.archive_branch(&tl, "summary-path").unwrap();
    assert_eq!(result.archived, vec![exclusive.clone()]);
    let t = db.timeline(&tl).unwrap();
    assert!(t.nodes.contains_key(&summary));
    assert!(!t.archive.contains_key(&summary));
    assert_eq!(
        t.context(&exclusive).unwrap().last().unwrap().text,
        "summary descendant"
    );
    assert_combined_invariants(t);
}

#[test]
fn archive_preconditions_still_protect_pins_and_current_branch_without_side_effects() {
    let mut f = archive_fixture();
    f.db.timeline_mut(&f.tl)
        .unwrap()
        .pin(&f.originals[2])
        .unwrap();
    let before = serde_json::to_value(&f.db).unwrap();
    let check = f.db.timeline(&f.tl).unwrap().archive_check("main").unwrap();
    assert!(!check.eligible);
    assert_eq!(check.reasons.len(), 1);
    assert!(check.reasons[0].contains("pin"));
    assert!(f.db.archive_branch(&f.tl, "main").is_err());
    assert_eq!(serde_json::to_value(&f.db).unwrap(), before);
    f.db.timeline_mut(&f.tl)
        .unwrap()
        .unpin(&f.originals[2])
        .unwrap();
    f.db.timeline_mut(&f.tl).unwrap().switch("main").unwrap();
    let before = serde_json::to_value(&f.db).unwrap();
    assert!(f.db.archive_branch(&f.tl, "main").is_err());
    assert_eq!(serde_json::to_value(&f.db).unwrap(), before);
}

#[test]
fn archived_watermark_and_cross_temperature_metadata_rebuild() {
    let mut f = archive_fixture();
    f.db.archive_branch(&f.tl, "main").unwrap();
    let clean = serde_json::to_value(&f.db).unwrap();
    let mut dirty = clean.clone();
    for location in ["nodes", "archive"] {
        for node in dirty["timelines"][&f.tl][location]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            node["prefix_tokens"] = json!("discard this cache");
            node["stale"] = json!(true);
        }
    }
    let mut rebuilt: Database = serde_json::from_value(dirty).unwrap();
    rebuilt.rebuild_and_validate().unwrap();
    assert_eq!(serde_json::to_value(&rebuilt).unwrap(), clean);
    assert_combined_invariants(rebuilt.timeline(&f.tl).unwrap());
    assert!(
        !rebuilt
            .timeline(&f.tl)
            .unwrap()
            .archive_check("live")
            .unwrap()
            .eligible
    );

    let before = serde_json::to_value(&rebuilt).unwrap();
    assert!(
        rebuilt
            .timeline_mut(&f.tl)
            .unwrap()
            .fork(&f.originals[3], "cold-head")
            .is_err()
    );
    assert_eq!(serde_json::to_value(&rebuilt).unwrap(), before);
    let mut bad_head = clean;
    bad_head["timelines"][&f.tl]["branches"]["live"] = json!(f.originals[3]);
    let mut rebuilt: Database = serde_json::from_value(bad_head).unwrap();
    assert!(rebuilt.rebuild_and_validate().is_err());
}

#[test]
fn hot_summary_can_extend_over_cold_parent_and_cold_ranges_can_be_revised() {
    let mut f = archive_fixture();
    f.db.archive_branch(&f.tl, "main").unwrap();
    let original_context =
        f.db.timeline(&f.tl)
            .unwrap()
            .context(&f.originals[3])
            .unwrap();
    f.db.timeline_mut(&f.tl)
        .unwrap()
        .fork(&f.dependent, "summary-work")
        .unwrap();
    f.db.timeline_mut(&f.tl)
        .unwrap()
        .switch("summary-work")
        .unwrap();
    let appended =
        f.db.append(&f.tl, "user", "work after summary", Some(17))
            .unwrap();
    let t = f.db.timeline(&f.tl).unwrap();
    assert_eq!(
        t.context(&appended)
            .unwrap()
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "original-0",
            "original-1",
            "depends on tail",
            "work after summary"
        ]
    );
    assert_eq!(t.tokens(&appended).unwrap(), 26);
    let budget = f.db.context_budget(&f.tl, &appended, 26).unwrap();
    assert_eq!(budget.total_tokens, 26);

    // New summary parent is archived; adding its child must preserve all prior children.
    let prior_children = f.db.timeline(&f.tl).unwrap().archive[&f.originals[1]]
        .children
        .clone();
    let cold_range =
        f.db.summarize(
            &f.tl,
            &f.originals[2],
            &f.originals[3],
            "cold tail",
            Some(2),
            vec![],
            None,
        )
        .unwrap();
    let parent = &f.db.timeline(&f.tl).unwrap().archive[&f.originals[1]];
    assert_eq!(&parent.children[..prior_children.len()], prior_children);
    assert!(parent.children.contains(&cold_range));
    let revision =
        f.db.summarize(
            &f.tl,
            &f.originals[1],
            &f.originals[3],
            "tail revision",
            Some(2),
            vec![],
            Some(f.summary.clone()),
        )
        .unwrap();
    let t = f.db.timeline(&f.tl).unwrap();
    assert!(t.node(&f.dependent).unwrap().stale);
    assert!(t.node(&f.downstream).unwrap().stale);
    assert_eq!(t.node(&revision).unwrap().summary_version, 2);
    assert_eq!(t.context(&f.originals[3]).unwrap(), original_context);
    assert_eq!(t.watermark.as_ref(), Some(&f.originals[3]));
    assert_combined_invariants(t);
    let snapshot = serde_json::to_value(&f.db).unwrap();
    let mut restored: Database = serde_json::from_value(snapshot.clone()).unwrap();
    restored.rebuild_and_validate().unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), snapshot);
    assert_combined_invariants(restored.timeline(&f.tl).unwrap());
}

#[test]
fn archived_storage_roundtrip_idempotency_and_rejected_mutation_preserve_snapshots() {
    let f = archive_fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.json");
    let backup = dir.path().join("archive.json.bak");
    let store = Storage::new(&path);
    store
        .transaction(&f.tl, None, |db| {
            *db = f.db.clone();
            Ok(Value::Null)
        })
        .unwrap();
    let result = store
        .transaction(&f.tl, Some("archive-once"), |db| {
            Ok(serde_json::to_value(db.archive_branch(&f.tl, "main")?).unwrap())
        })
        .unwrap();
    assert_eq!(result["archived"], json!(f.originals[1..]));
    let before = fs::read(&path).unwrap();
    let backup_before = fs::read(&backup).unwrap();
    let reloaded_store = Storage::new(&path);
    let repeated = reloaded_store
        .transaction(&f.tl, Some("archive-once"), |_| {
            panic!("a repeated key must not archive a deleted branch twice")
        })
        .unwrap();
    assert_eq!(repeated, result);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&backup).unwrap(), backup_before);
    assert!(
        reloaded_store
            .transaction(&f.tl, Some("invalid-archive"), |db| {
                Ok(serde_json::to_value(db.archive_branch(&f.tl, "live")?).unwrap())
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&backup).unwrap(), backup_before);
    let loaded = reloaded_store.read().unwrap();
    assert_combined_invariants(loaded.timeline(&f.tl).unwrap());
    assert_eq!(
        loaded
            .audit
            .iter()
            .filter(|a| a.op == "archive_branch")
            .count(),
        1
    );
    assert_eq!(loaded.idempotency.len(), 1);
    reloaded_store
        .transaction(&f.tl, Some("continue"), |db| {
            db.timeline_mut(&f.tl)?.fork(&f.dependent, "continue")?;
            db.timeline_mut(&f.tl)?.switch("continue")?;
            Ok(json!({"node": db.append(&f.tl, "user", "persist after archive", Some(2))?}))
        })
        .unwrap();
    assert_combined_invariants(reloaded_store.read().unwrap().timeline(&f.tl).unwrap());
}

#[test]
fn randomized_archive_splits_preserve_graph_contexts_and_watermarks() {
    let mut seed = 0xa7_c4_1e_11_12_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..128 {
        let count = 4 + (next() % 10) as usize;
        let costs: Vec<_> = (0..count).map(|_| 1 + next() % 80).collect();
        let (mut db, tl, ids) = chain(&costs);
        let split = (next() % (count - 2) as u64) as usize;
        let held = split + 1 + (next() % (count - split - 2) as u64) as usize;
        db.timeline_mut(&tl)
            .unwrap()
            .fork(&ids[split], "live")
            .unwrap();
        db.timeline_mut(&tl)
            .unwrap()
            .fork(&ids[held], "held")
            .unwrap();
        db.timeline_mut(&tl).unwrap().switch("live").unwrap();
        db.append(&tl, "assistant", "live extension", Some(1 + next() % 80))
            .unwrap();
        db.summarize(
            &tl,
            &ids[0],
            &ids[count - 1],
            "whole history",
            Some(3),
            vec![],
            None,
        )
        .unwrap();
        let middle = split + 1 + (next() % (count - split - 1) as u64) as usize;
        let nested = db
            .summarize(
                &tl,
                &ids[middle],
                &ids[middle],
                "one segment",
                Some(1),
                vec![],
                None,
            )
            .unwrap();
        let original = all_nodes(db.timeline(&tl).unwrap());
        let context = db.timeline(&tl).unwrap().context(&ids[count - 1]).unwrap();
        let watermark = db.timeline(&tl).unwrap().watermark.clone();
        assert_eq!(
            db.archive_branch(&tl, "main").unwrap().archived,
            ids[held + 1..]
        );
        assert_combined_invariants(db.timeline(&tl).unwrap());
        assert_eq!(
            db.archive_branch(&tl, "held").unwrap().archived,
            ids[split + 1..=held]
        );
        let t = db.timeline(&tl).unwrap();
        assert_eq!(all_nodes(t), original);
        assert_eq!(t.context(&ids[count - 1]).unwrap(), context);
        assert_eq!(t.watermark, watermark);
        assert!(t.nodes.contains_key(&nested));
        assert_combined_invariants(t);
        let serialized = serde_json::to_value(&db).unwrap();
        let mut restored: Database = serde_json::from_value(serialized.clone()).unwrap();
        restored.rebuild_and_validate().unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), serialized);
        assert_combined_invariants(restored.timeline(&tl).unwrap());
    }
}

#[test]
fn infeasible_budget_reports_exact_minimum_and_preserves_entire_database() {
    for (costs, pin, budget, required) in [
        (vec![1000, 1, 1000, 0], Some(1), 1, 65),
        (vec![1000, 1, 2, 0], Some(1), 1, 35),
        (vec![100], None, 0, 100),
    ] {
        let (mut db, tl, ids) = chain(&costs);
        if let Some(index) = pin {
            db.timeline_mut(&tl).unwrap().pin(&ids[index]).unwrap();
        }
        let before = serde_json::to_value(&db).unwrap();
        let error = db
            .context_budget(&tl, ids.last().unwrap(), budget)
            .unwrap_err();
        assert_eq!(error.message, "budget infeasible");
        assert_eq!(error.to_string(), "budget infeasible");
        assert_eq!(
            error.data,
            Some(json!({"required_tokens":required,"max_tokens":budget}))
        );
        assert_eq!(serde_json::to_value(&db).unwrap(), before);
    }
}
