use serde_json::{Value, json};
use std::fs;
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};
use timeline::storage::{IDEMPOTENCY_TTL_SECONDS, Storage};
use timeline::{AuditEntry, Database, Error, IdempotencyEntry};

fn legacy_snapshot() -> Value {
    json!({
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
    })
}

#[test]
fn absent_database_roundtrip_and_backup_are_complete_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conversations.json");
    let storage = Storage::new(&path);
    assert!(storage.read().unwrap().timelines.is_empty());
    assert!(!path.exists());
    storage
        .transaction("tl0", None, |db| {
            *db = serde_json::from_value(legacy_snapshot()).unwrap();
            Ok(json!({"created":true}))
        })
        .unwrap();
    let original = fs::read(&path).unwrap();
    assert_eq!(storage.read().unwrap().timelines["tl0"].nodes.len(), 2);
    storage
        .transaction("tl0", None, |db| {
            db.next_node += 1;
            Ok(json!({"updated":true}))
        })
        .unwrap();
    assert_eq!(
        fs::read(path.with_file_name("conversations.json.bak")).unwrap(),
        original
    );
    assert_eq!(storage.read().unwrap().next_node, 3);
    assert_eq!(
        Storage::new(path.with_file_name("conversations.json.bak"))
            .read()
            .unwrap()
            .next_node,
        2
    );
}

#[test]
fn legacy_format_defaults_and_damaged_derived_fields_rebuild() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.json");
    let mut fixture = legacy_snapshot();
    fixture["timelines"]["tl0"]["nodes"]["n0"]["prefix_tokens"] = json!("corrupt derived cache");
    fixture["timelines"]["tl0"]["nodes"]["n1"]["prefix_tokens"] = json!({"bad":true});
    fixture["timelines"]["tl0"]["nodes"]["n1"]["stale"] = json!(["bad"]);
    fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    let storage = Storage::new(path);
    let db = storage.read().unwrap();
    let timeline = &db.timelines["tl0"];
    assert_eq!(timeline.nodes["n0"].prefix_tokens, 0);
    assert_eq!(timeline.nodes["n1"].prefix_tokens, 5);
    assert_eq!(timeline.nodes["n1"].summary_version, 1);
    assert!(!timeline.nodes["n1"].stale);
    assert!(timeline.watermark.is_none());
    assert!(timeline.archive.is_empty());
    assert!(db.audit.is_empty());
    assert!(db.idempotency.is_empty());
    storage
        .transaction("tl0", None, |_| Ok(json!(true)))
        .unwrap();
    let reloaded = storage.read().unwrap();
    assert_eq!(reloaded.timelines["tl0"].nodes["n1"].prefix_tokens, 5);
}

#[test]
fn corrupted_or_incompatible_source_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private-internal-path.json");
    let storage = Storage::new(&path);
    let mut unsupported = legacy_snapshot();
    unsupported["format_version"] = json!(999);
    let mut broken_parent = legacy_snapshot();
    broken_parent["timelines"]["tl0"]["nodes"]["n1"]["parent"] = json!("n999");
    for bytes in [
        b"{broken JSON".to_vec(),
        serde_json::to_vec(&unsupported).unwrap(),
        serde_json::to_vec(&broken_parent).unwrap(),
    ] {
        fs::write(&path, &bytes).unwrap();
        let read_error = storage.read().unwrap_err().to_string();
        assert!(read_error.contains(".bak"));
        assert!(!read_error.contains("private-internal-path"));
        let result = storage.transaction("tl0", None, |_| {
            panic!("invalid snapshots must be rejected before invoking the operation")
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(
            !path
                .with_file_name("private-internal-path.json.bak")
                .exists()
        );
    }
}

#[test]
fn operation_failure_rolls_back_every_field_and_preserves_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.json");
    let storage = Storage::new(&path);
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    let original = fs::read(&path).unwrap();
    let error = storage.transaction("scope", Some("failed"), |db| {
        db.next_node += 99;
        Err(Error::new("intentional rejected operation".into()))
    });
    assert!(error.is_err());
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(storage.read().unwrap().idempotency.is_empty());
}

#[test]
fn parallel_transactions_and_readers_do_not_lose_updates() {
    const WRITERS: usize = 6;
    const WRITES: u64 = 12;
    let directory = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(directory.path().join("parallel.json")));
    let barrier = Arc::new(Barrier::new(WRITERS + 1));
    let mut workers = Vec::new();
    for _ in 0..WRITERS {
        let storage = Arc::clone(&storage);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..WRITES {
                storage
                    .transaction("scope", None, |db| {
                        db.next_node += 1;
                        Ok(json!(db.next_node))
                    })
                    .unwrap();
                // Readers participate in the same locking protocol.
                storage.read().unwrap();
            }
        }));
    }
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(storage.read().unwrap().next_node, WRITERS as u64 * WRITES);
}

#[test]
fn independent_cli_processes_serialize_the_full_read_modify_write_cycle() {
    use std::process::{Command, Stdio};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("processes.json");
    let created = Command::new(env!("CARGO_BIN_EXE_tl"))
        .arg("--db")
        .arg(&path)
        .arg("create")
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created: Value = serde_json::from_slice(&created.stdout).unwrap();
    let id = created["timeline_id"].as_str().unwrap();
    let mut children = Vec::new();
    for index in 0..8 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_tl"))
                .arg("--db")
                .arg(&path)
                .arg("--timeline")
                .arg(id)
                .args(["append", "--role", "user", "--tokens", "1", "--text"])
                .arg(format!("writer-{index}"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let mut results = std::collections::HashSet::new();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(results.insert(output["node"].as_str().unwrap().to_owned()));
    }
    let db = Storage::new(path).read().unwrap();
    assert_eq!(db.timelines[id].nodes.len(), 9);
    assert_eq!(db.next_node, 9);
    assert_eq!(
        db.timelines[id]
            .context(&db.timelines[id].branches["main"])
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn idempotency_is_persistent_scoped_collision_safe_and_expires() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keys.json");
    let storage = Storage::new(&path);
    let first = storage
        .transaction("a:b", Some("c"), |db| {
            db.next_node += 1;
            Ok(json!({"node":format!("n{}", db.next_node)}))
        })
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    let reopened = Storage::new(&path);
    let duplicate = reopened
        .transaction("a:b", Some("c"), |_| {
            panic!("a duplicate key must not execute its operation")
        })
        .unwrap();
    assert_eq!(first, duplicate);
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "dedup hits do not write a snapshot"
    );
    reopened
        .transaction("a", Some("b:c"), |db| {
            db.next_node += 1;
            Ok(json!({"node":format!("n{}", db.next_node)}))
        })
        .unwrap();
    assert_eq!(
        storage.read().unwrap().next_node,
        2,
        "scopes cannot collide on delimiters"
    );
    assert_eq!(
        storage
            .read()
            .unwrap()
            .audit
            .iter()
            .filter(|entry| entry.op == "idempotency_key")
            .count(),
        2
    );

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Simulate a persisted snapshot reopening seven days later without sleeping
    // or mutating the process-wide clock/environment.
    let mut aged = storage.read().unwrap();
    aged.idempotency.insert(
        json!(["old", "expired"]).to_string(),
        IdempotencyEntry {
            created_at: now - IDEMPOTENCY_TTL_SECONDS,
            result: json!("old result"),
        },
    );
    fs::write(&path, serde_json::to_vec(&aged).unwrap()).unwrap();
    let fresh = storage
        .transaction("old", Some("expired"), |db| {
            db.next_node += 1;
            Ok(json!("new result"))
        })
        .unwrap();
    assert_eq!(fresh, json!("new result"));
    assert_eq!(storage.read().unwrap().next_node, 3);
}

#[test]
fn failed_backup_preserves_current_database_and_removes_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("safe.json");
    let storage = Storage::new(&path);
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    let original = fs::read(&path).unwrap();
    // An occupied directory is an unambiguous rename failure on both platforms.
    fs::create_dir(directory.path().join("safe.json.bak")).unwrap();
    assert!(
        storage
            .transaction("scope", None, |db| {
                db.next_node += 1;
                Ok(json!(null))
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(storage.read().unwrap().next_node, 0);
    assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".timeline-")
    }));
}

#[test]
fn concurrent_duplicate_key_executes_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(directory.path().join("dedup.json")));
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                storage
                    .transaction("same", Some("exactly-once"), |db| {
                        db.next_node += 1;
                        Ok(json!(db.next_node))
                    })
                    .unwrap()
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), json!(1));
    }
    assert_eq!(storage.read().unwrap().next_node, 1);
    assert_eq!(storage.read().unwrap().idempotency.len(), 1);
}

#[test]
fn storage_supplies_audit_timestamps_without_rewriting_history() {
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::new(directory.path().join("audit.json"));
    let push = |db: &mut Database| {
        db.audit.push(AuditEntry {
            ts: 0,
            op: "test".into(),
            timeline_id: "scope".into(),
            from: None,
            to: None,
            node: None,
            details: json!({"evidence":true}),
        });
        Ok(json!(null))
    };
    storage.transaction("scope", None, push).unwrap();
    let first = storage.read().unwrap().audit[0].ts;
    assert!(first > 0);
    storage.transaction("scope", None, push).unwrap();
    let db = storage.read().unwrap();
    assert_eq!(db.audit[0].ts, first);
    assert!(db.audit[1].ts >= first);
    assert!(
        storage
            .transaction("scope", None, |db| {
                db.audit.clear();
                Ok(json!(null))
            })
            .is_err()
    );
    assert!(
        storage
            .transaction("scope", None, |db| {
                db.audit[0].op = "rewritten".into();
                Ok(json!(null))
            })
            .is_err()
    );
    assert_eq!(storage.read().unwrap().audit.len(), 2);
    assert_eq!(storage.read().unwrap().audit[0].op, "test");
}

#[cfg(unix)]
#[test]
fn unix_snapshots_backups_and_locks_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private.json");
    let storage = Storage::new(&path);
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    for name in ["private.json", "private.json.bak", "private.json.lock"] {
        assert_eq!(
            fs::metadata(directory.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_snapshots_backups_and_locks_have_protected_owner_only_dacl() {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
    let directory = tempfile::tempdir().unwrap();
    let storage = Storage::new(directory.path().join("private.json"));
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    storage
        .transaction("scope", None, |_| Ok(json!(null)))
        .unwrap();
    for name in ["private.json", "private.json.bak", "private.json.lock"] {
        let path = directory.path().join(name);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut descriptor = null_mut();
        let mut sddl = null_mut();
        let mut length = 0;
        // SAFETY: Windows allocates both buffers; each is inspected while valid
        // and released once with LocalFree. All optional outputs are null.
        unsafe {
            assert_eq!(
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor
                ),
                0
            );
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut sddl,
                    &mut length
                ),
                0
            );
            let text =
                String::from_utf16_lossy(std::slice::from_raw_parts(sddl, length as usize - 1));
            LocalFree(sddl.cast());
            LocalFree(descriptor);
            assert_eq!(text, "D:P(A;;FA;;;OW)", "unexpected DACL on {name}");
        }
    }
}
