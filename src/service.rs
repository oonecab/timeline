//! Shared, validated JSON interface used by the CLI and MCP transports.

use crate::storage::Storage;
use crate::{Database, Error, Intent, MAX_TEXT_BYTES, Message, Result};
use serde_json::{Map, Value, json};

pub const TOOL_NAMES: [&str; 14] = [
    "timeline_create",
    "timeline_append",
    "timeline_fork",
    "timeline_switch",
    "timeline_drop_branch",
    "timeline_context",
    "timeline_context_budget",
    "timeline_pin",
    "timeline_unpin",
    "timeline_summarize",
    "timeline_branches",
    "timeline_node",
    "timeline_archive_check",
    "timeline_archive_branch",
];

/// Allows a 1 MiB message even when every byte is JSON-escaped as `\uXXXX`.
pub const MAX_MCP_REQUEST_BYTES: usize = 8 * 1024 * 1024;

fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(format!("{key} must be a string")))
}

fn optional_tokens(args: &Value, key: &str) -> Result<Option<u64>> {
    args.get(key)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| Error::new(format!("{key} must be a nonnegative 64-bit integer")))
        })
        .transpose()
}

/// Spec §3.1/§A(a): output messages carry `intent_tag` only when the
/// originating node is annotated. The field is memory-only on `Message`
/// (never persisted), so the transport shapes it explicitly.
fn message_json(message: &Message) -> Value {
    let mut value = json!({"role": message.role, "text": message.text, "tokens": message.tokens});
    if let Some(tag) = &message.intent_tag {
        value["intent_tag"] = json!(tag);
    }
    value
}

fn optional_intent(args: &Value) -> Result<Option<Intent>> {
    let Some(value) = args.get("intent") else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| Error::new("intent must be an object with a tag".into()))?;
    let tag = object
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("intent.tag must be a string".into()))?;
    if tag.is_empty() {
        return Err(Error::new("intent.tag must be nonempty".into()));
    }
    let note = object
        .get("note")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Some(Intent {
        tag: tag.to_owned(),
        note,
    }))
}

fn validate_id(value: &str, prefix: &str, key: &str) -> Result<()> {
    let suffix = value.strip_prefix(prefix).unwrap_or("");
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::new(format!("{key} must match {prefix}[0-9]+")));
    }
    Ok(())
}

fn fields(tool: &str) -> Result<(&'static [&'static str], &'static [&'static str])> {
    Ok(match tool {
        "timeline_create" => (&[], &[]),
        "timeline_append" => (&["role", "text"], &["tokens", "intent"]),
        "timeline_fork" => (&["node", "name"], &[]),
        "timeline_switch" | "timeline_drop_branch" => (&["name"], &[]),
        "timeline_context" | "timeline_pin" | "timeline_unpin" | "timeline_node" => {
            (&["node"], &[])
        }
        "timeline_context_budget" => (&["node", "max_tokens"], &[]),
        "timeline_summarize" => (
            &["from", "to", "summary"],
            &["tokens", "derived_from", "supersede", "intent"],
        ),
        "timeline_branches" => (&[], &[]),
        "timeline_archive_check" | "timeline_archive_branch" => (&["branch"], &[]),
        _ => return Err(Error::new("unknown timeline tool".into())),
    })
}

pub fn validate_arguments(tool: &str, args: &Value) -> Result<()> {
    let object = args
        .as_object()
        .ok_or_else(|| Error::new("arguments must be an object".into()))?;
    let (required, optional) = fields(tool)?;
    for key in required {
        if !object.contains_key(*key) {
            return Err(Error::new(format!("missing required argument: {key}")));
        }
    }
    if tool != "timeline_create" {
        validate_id(string(args, "timeline_id")?, "tl", "timeline_id")?;
    }
    for key in object.keys() {
        if !required.contains(&key.as_str())
            && !optional.contains(&key.as_str())
            && key != "idempotency_key"
            && !(key == "timeline_id" && tool != "timeline_create")
        {
            return Err(Error::new(format!("unknown argument: {key}")));
        }
        match key.as_str() {
            "tokens" | "max_tokens" => {
                optional_tokens(args, key)?;
            }
            "node" | "from" | "to" | "supersede" => validate_id(string(args, key)?, "n", key)?,
            "name" | "branch" => {
                let name = string(args, key)?;
                if name.is_empty() || name.contains('/') {
                    return Err(Error::new(
                        "branch name must be nonempty and cannot contain /".into(),
                    ));
                }
            }
            "role" => {
                if !["user", "assistant", "system", "summary"].contains(&string(args, key)?) {
                    return Err(Error::new(
                        "role must be user, assistant, system, or summary".into(),
                    ));
                }
            }
            "derived_from" => {
                let entries = args[key].as_array().ok_or_else(|| {
                    Error::new("derived_from must be an array of node IDs".into())
                })?;
                for entry in entries {
                    validate_id(
                        entry.as_str().ok_or_else(|| {
                            Error::new("derived_from must contain only node IDs".into())
                        })?,
                        "n",
                        key,
                    )?;
                }
            }
            "text" | "summary" => {
                if string(args, key)?.len() > MAX_TEXT_BYTES {
                    return Err(Error::new("message text exceeds 1 MiB".into()));
                }
            }
            "intent" => {
                // Spec §10: tag must be a nonempty string, note any string.
                // The tag vocabulary is governed by the skill layer and is
                // deliberately not enum-validated here.
                let object = args[key].as_object().ok_or_else(|| {
                    Error::new("intent must be an object with a tag".into())
                })?;
                for field in object.keys() {
                    if field != "tag" && field != "note" {
                        return Err(Error::new(format!("unknown intent field: {field}")));
                    }
                }
                let tag = object
                    .get("tag")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("intent.tag must be a string".into()))?;
                if tag.is_empty() {
                    return Err(Error::new("intent.tag must be nonempty".into()));
                }
                if let Some(note) = object.get("note") {
                    if !note.is_string() {
                        return Err(Error::new("intent.note must be a string".into()));
                    }
                }
            }
            _ => {
                string(args, key)?;
            }
        }
    }
    Ok(())
}

pub fn is_mutating(tool: &str) -> bool {
    !matches!(
        tool,
        "timeline_context" | "timeline_branches" | "timeline_node" | "timeline_archive_check"
    )
}

/// Run one transport request under the appropriate storage lock.
pub fn call_storage(storage: &Storage, tool: &str, args: &Value) -> Result<Value> {
    validate_arguments(tool, args)?;
    if is_mutating(tool) {
        let scope = args
            .get("timeline_id")
            .and_then(Value::as_str)
            .unwrap_or("__create__");
        let key = args.get("idempotency_key").and_then(Value::as_str);
        storage.transaction(scope, key, |db| dispatch(db, tool, args))
    } else {
        dispatch(&mut storage.read()?, tool, args)
    }
}

pub fn dispatch(db: &mut Database, tool: &str, args: &Value) -> Result<Value> {
    validate_arguments(tool, args)?;
    if tool == "timeline_create" {
        return Ok(json!({"timeline_id": db.create()?}));
    }
    let timeline_id = string(args, "timeline_id")?;
    let result = match tool {
        "timeline_append" => {
            json!({"node": db.append_with_intent(timeline_id, string(args, "role")?, string(args, "text")?, optional_tokens(args, "tokens")?, optional_intent(args)?)?})
        }
        "timeline_fork" => {
            let node = string(args, "node")?;
            let name = string(args, "name")?;
            db.timeline_mut(timeline_id)?.fork(node, name)?;
            json!({"branch": name, "head": node})
        }
        "timeline_switch" => {
            let name = string(args, "name")?;
            db.timeline_mut(timeline_id)?.switch(name)?;
            json!({"current_branch": name})
        }
        "timeline_drop_branch" => {
            let name = string(args, "name")?;
            db.timeline_mut(timeline_id)?.drop_branch(name)?;
            json!({"dropped": name})
        }
        "timeline_context" => {
            let messages = db
                .timeline(timeline_id)?
                .context(string(args, "node")?)?;
            json!(messages.iter().map(message_json).collect::<Vec<_>>())
        }
        "timeline_context_budget" => {
            let result = db.context_budget(
                timeline_id,
                string(args, "node")?,
                optional_tokens(args, "max_tokens")?.expect("validated required max_tokens"),
            )?;
            json!({
                "messages": result.messages.iter().map(message_json).collect::<Vec<_>>(),
                "total_tokens": result.total_tokens,
                "summaries_created": result.summaries_created,
            })
        }
        "timeline_pin" => {
            let node = string(args, "node")?;
            db.timeline_mut(timeline_id)?.pin(node)?;
            json!({"pinned": node})
        }
        "timeline_unpin" => {
            let node = string(args, "node")?;
            db.timeline_mut(timeline_id)?.unpin(node)?;
            json!({"unpinned": node})
        }
        "timeline_summarize" => {
            let derived = args
                .get("derived_from")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .map(|entry| entry.as_str().expect("validated derived_from").to_owned())
                        .collect()
                })
                .unwrap_or_default();
            let supersede = args
                .get("supersede")
                .and_then(Value::as_str)
                .map(str::to_owned);
            json!({"node": db.summarize_with_intent(timeline_id, string(args, "from")?, string(args, "to")?, string(args, "summary")?, optional_tokens(args, "tokens")?, derived, supersede, optional_intent(args)?)?})
        }
        "timeline_branches" => json!(db.timeline(timeline_id)?.branches()),
        "timeline_node" => {
            let node = db.timeline(timeline_id)?.node(string(args, "node")?)?;
            json!({
                "id": node.id, "parent": node.parent, "children": node.children,
                "role": node.message.as_ref().map(|message| &message.role),
                "text": node.message.as_ref().map(|message| &message.text),
                "tokens": node.message.as_ref().map_or(0, |message| message.tokens),
                "pinned": node.pinned, "prefix_tokens": node.prefix_tokens,
                "summary_range": node.summary_range, "summary_version": node.summary_version,
                "supersedes": node.supersedes, "derived_from": node.derived_from, "stale": node.stale,
                "intent": node.intent,
            })
        }
        "timeline_archive_check" => json!(
            db.timeline(timeline_id)?
                .archive_check(string(args, "branch")?)?
        ),
        "timeline_archive_branch" => {
            json!(db.archive_branch(timeline_id, string(args, "branch")?)?)
        }
        _ => return Err(Error::new("unknown timeline tool".into())),
    };
    Ok(result)
}

pub fn tool_definitions() -> Vec<Value> {
    TOOL_NAMES.iter().map(|name| {
        let (required_fields, optional_fields) = fields(name).expect("known tool");
        let mut properties = Map::new();
        let mut required = Vec::new();
        if *name != "timeline_create" {
            properties.insert("timeline_id".into(), json!({"type":"string", "pattern":"^tl[0-9]+$"}));
            required.push("timeline_id");
        }
        required.extend(required_fields.iter().copied());
        for key in required_fields.iter().chain(optional_fields.iter()).copied() {
            properties.insert(key.to_owned(), match key {
                "tokens" | "max_tokens" => json!({"type":"integer", "minimum":0, "maximum":u64::MAX}),
                "node" | "from" | "to" | "supersede" => json!({"type":"string", "pattern":"^n[0-9]+$"}),
                "role" => json!({"type":"string", "enum":["user", "assistant", "system", "summary"]}),
                "name" | "branch" => json!({"type":"string", "minLength":1, "pattern":"^[^/]+$"}),
                "derived_from" => json!({"type":"array", "items":{"type":"string", "pattern":"^n[0-9]+$"}}),
                "intent" => json!({"type":"object", "properties":{"tag":{"type":"string","minLength":1},"note":{"type":"string"}}, "required":["tag"], "additionalProperties":false, "description":"Optional intent annotation recorded at write time; only its tag is echoed in context output as intent_tag."}),
                "text" | "summary" => json!({"type":"string", "description":"At most 1 MiB of UTF-8 text."}),
                _ => json!({"type":"string"}),
            });
        }
        properties.insert("idempotency_key".into(), json!({"type":"string", "description":"Optional key for replaying a mutation once per timeline for seven days."}));
        json!({"name": name, "description": match *name {
            "timeline_create" => "Create a timeline with a root and main branch.",
            "timeline_append" => "Append a message to the current branch.",
            "timeline_fork" => "Create a named branch at an existing node without copying history.",
            "timeline_switch" => "Switch the current branch.",
            "timeline_drop_branch" => "Remove a noncurrent branch pointer while retaining its nodes.",
            "timeline_context" => "Return the full root-to-node message path in chronological order.",
            "timeline_context_budget" => "Assemble context with target and pin priority and reversible summaries.",
            "timeline_pin" => "Always include a node in budgeted context.",
            "timeline_unpin" => "Remove a node's pin while preserving watermark integrity.",
            "timeline_summarize" => "Create a reversible summary or a new revision without deleting original text.",
            "timeline_branches" => "List the timeline's branch pointers.",
            "timeline_node" => "Inspect a node and its summary metadata.",
            "timeline_archive_check" => "Check watermark, pin, and placeholder conditions for archiving.",
            "timeline_archive_branch" => "Archive an eligible noncurrent branch's exclusive nodes.",
            _ => unreachable!(),
        }, "inputSchema":{"type":"object", "properties":properties, "required":required, "additionalProperties":false}})
    }).collect()
}
