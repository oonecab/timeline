use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use timeline::MAX_TEXT_BYTES;
use timeline::service::{MAX_MCP_REQUEST_BYTES, TOOL_NAMES};

fn cli_output(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tl"))
        .arg("--db")
        .arg(db)
        .args(args)
        .env_remove("TIMELINE_DB")
        .output()
        .expect("run tl")
}

fn cli(db: &Path, args: &[&str]) -> Value {
    let output = cli_output(db, args);
    assert!(
        output.status.success(),
        "tl {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "success must not print stderr");
    serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON value")
}

fn cli_error(db: &Path, args: &[&str]) -> String {
    let output = cli_output(db, args);
    assert!(
        !output.status.success(),
        "tl {args:?} unexpectedly succeeded"
    );
    assert!(output.stdout.is_empty(), "errors must not print stdout");
    let result: Value = serde_json::from_slice(&output.stderr).expect("stderr is JSON");
    result["error"].as_str().expect("error reason").to_owned()
}

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    called: BTreeSet<String>,
}

impl Mcp {
    fn from_command(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start MCP process");
        Self {
            stdin: child.stdin.take(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
            next_id: 0,
            called: BTreeSet::new(),
        }
    }

    fn new(db: &Path) -> Self {
        Self::from_command(
            Command::new(env!("CARGO_BIN_EXE_tl-mcp"))
                .arg("--db")
                .arg(db)
                .env_remove("TIMELINE_DB"),
        )
    }

    fn raw(&mut self, text: &str) -> Value {
        self.raw_bytes(text.as_bytes())
    }

    fn raw_bytes(&mut self, bytes: &[u8]) -> Value {
        self.stdin
            .as_mut()
            .expect("stdin open")
            .write_all(bytes)
            .unwrap();
        self.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
        self.stdin.as_mut().unwrap().flush().unwrap();
        let mut line = String::new();
        assert!(
            self.stdout.read_line(&mut line).unwrap() > 0,
            "MCP exited before replying"
        );
        serde_json::from_str(&line).expect("MCP stdout must contain only JSON-RPC")
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let response = self.raw(
            &json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params}).to_string(),
        );
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], self.next_id);
        response
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.called.insert(name.to_owned());
        let response = self.request("tools/call", json!({"name":name,"arguments":arguments}));
        assert!(response.get("error").is_none(), "{name}: {response}");
        assert_eq!(response["result"]["isError"], false);
        serde_json::from_str(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("text content"),
        )
        .unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.wait().is_err() {
            let _ = self.child.kill();
        }
    }
}

#[test]
fn cli_all_commands_and_json_shapes() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("all-cli.json");
    assert_eq!(cli(&db, &["create"]), json!({"timeline_id":"tl0"}));
    let one = cli(
        &db,
        &[
            "append",
            "--role",
            "user",
            "--text",
            "引理2看不懂",
            "--tokens",
            "20",
        ],
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    let two = cli(
        &db,
        &[
            "append",
            "--role",
            "assistant",
            "--text",
            "我们一步步来",
            "--tokens",
            "30",
        ],
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cli(&db, &["fork", "--node", &one, "--name", "追问"]),
        json!({"branch":"追问","head":one})
    );
    assert_eq!(
        cli(&db, &["switch", "--name", "追问"]),
        json!({"current_branch":"追问"})
    );
    let alternate = cli(
        &db,
        &[
            "append",
            "--role",
            "assistant",
            "--text",
            "换个证明",
            "--tokens",
            "15",
        ],
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(cli(&db, &["branches"]).as_array().unwrap().len(), 2);
    let context = cli(&db, &["context", "--node", &alternate]);
    assert_eq!(
        context,
        json!([{"role":"user","text":"引理2看不懂","tokens":20},{"role":"assistant","text":"换个证明","tokens":15}])
    );
    let budget = cli(&db, &["budget", "--node", &alternate, "--max", "100"]);
    assert_eq!(budget["messages"], context);
    assert_eq!(budget["total_tokens"], 35);
    assert_eq!(cli(&db, &["pin", "--node", &one]), json!({"pinned":one}));
    let node = cli(&db, &["node", "--node", &one]);
    assert_eq!(node["pinned"], true);
    assert_eq!(node["role"], "user");
    assert_eq!(node["text"], "引理2看不懂");
    assert_eq!(node["tokens"], 20);
    assert_eq!(node["prefix_tokens"], 20);
    assert!(
        node.get("message").is_none(),
        "transport node output is flattened"
    );
    assert_eq!(
        cli(&db, &["unpin", "--node", &one]),
        json!({"unpinned":one})
    );
    let summary = cli(
        &db,
        &[
            "summarize",
            "--from",
            &one,
            "--to",
            &two,
            "--summary",
            "讨论引理2",
            "--tokens",
            "8",
        ],
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    let revision = cli(
        &db,
        &[
            "summarize",
            "--from",
            &one,
            "--to",
            &two,
            "--summary",
            "修正讨论",
            "--supersede",
            &summary,
        ],
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cli(&db, &["node", "--node", &revision])["summary_version"],
        2
    );
    cli(&db, &["fork", "--node", &two, "--name", "旧分支"]);
    assert_eq!(
        cli(&db, &["archive-check", "--name", "旧分支"]),
        json!({"eligible":true,"reasons":[]})
    );
    assert_eq!(
        cli(&db, &["archive-branch", "--name", "旧分支"]),
        json!({"archived":[]})
    );
    cli(&db, &["switch", "--name", "main"]);
    assert_eq!(
        cli(&db, &["drop-branch", "--name", "追问"]),
        json!({"dropped":"追问"})
    );
    assert_eq!(
        cli(&db, &["context", "--node", &two])
            .as_array()
            .unwrap()
            .len(),
        2,
        "summary never removes original text"
    );
    assert!(!cli_error(&db, &["fork", "--node", "n99999", "--name", "missing"]).is_empty());
}

#[test]
fn mcp_initialize_all_fourteen_tools_and_errors() {
    let temp = tempfile::tempdir().unwrap();
    let mut mcp = Mcp::new(&temp.path().join("all-mcp.json"));
    let initialize = mcp.request("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"acceptance","version":"1"}}));
    assert_eq!(
        initialize["result"]["serverInfo"],
        json!({"name":"timeline","version":"1.1.0"})
    );
    assert_eq!(initialize["result"]["protocolVersion"], "2024-11-05");
    writeln!(
        mcp.stdin.as_mut().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    assert_eq!(
        mcp.request("ping", json!({}))["result"],
        json!({}),
        "notification must produce no extra response"
    );
    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 14);
    let advertised: BTreeSet<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        advertised,
        TOOL_NAMES.iter().map(|name| name.to_string()).collect()
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            tool["inputSchema"]["properties"]["idempotency_key"]["type"],
            "string"
        );
        if tool["name"] != "timeline_create" {
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("timeline_id"))
            );
        }
    }
    let tl = mcp.call("timeline_create", json!({}))["timeline_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let one = mcp.call(
        "timeline_append",
        json!({"timeline_id":tl,"role":"user","text":"first","tokens":20}),
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    let two = mcp.call(
        "timeline_append",
        json!({"timeline_id":tl,"role":"assistant","text":"second","tokens":30}),
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        mcp.call(
            "timeline_fork",
            json!({"timeline_id":tl,"node":one,"name":"旁支"})
        )["head"],
        one
    );
    assert_eq!(
        mcp.call("timeline_switch", json!({"timeline_id":tl,"name":"旁支"}))["current_branch"],
        "旁支"
    );
    let alternate = mcp.call(
        "timeline_append",
        json!({"timeline_id":tl,"role":"assistant","text":"alternate","tokens":10}),
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        mcp.call("timeline_branches", json!({"timeline_id":tl}))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let context = mcp.call(
        "timeline_context",
        json!({"timeline_id":tl,"node":alternate}),
    );
    assert_eq!(context[0]["text"], "first");
    assert_eq!(context[1]["text"], "alternate");
    assert_eq!(
        mcp.call(
            "timeline_context_budget",
            json!({"timeline_id":tl,"node":alternate,"max_tokens":100})
        )["messages"],
        context
    );
    assert_eq!(
        mcp.call("timeline_pin", json!({"timeline_id":tl,"node":one}))["pinned"],
        one
    );
    assert_eq!(
        mcp.call("timeline_node", json!({"timeline_id":tl,"node":one}))["pinned"],
        true
    );
    assert_eq!(
        mcp.call("timeline_unpin", json!({"timeline_id":tl,"node":one}))["unpinned"],
        one
    );
    let summary = mcp.call(
        "timeline_summarize",
        json!({"timeline_id":tl,"from":one,"to":two,"summary":"a real summary","tokens":5}),
    )["node"]
        .as_str()
        .unwrap()
        .to_owned();
    let dependent = mcp.call("timeline_summarize", json!({"timeline_id":tl,"from":one,"to":alternate,"summary":"derived","tokens":5,"derived_from":[summary]}))["node"].as_str().unwrap().to_owned();
    let revision = mcp.call("timeline_summarize", json!({"timeline_id":tl,"from":one,"to":two,"summary":"corrected","tokens":6,"supersede":summary}))["node"].as_str().unwrap().to_owned();
    assert_eq!(
        mcp.call("timeline_node", json!({"timeline_id":tl,"node":revision}))["summary_version"],
        2
    );
    assert_eq!(
        mcp.call("timeline_node", json!({"timeline_id":tl,"node":dependent}))["stale"],
        true
    );
    mcp.call(
        "timeline_fork",
        json!({"timeline_id":tl,"node":two,"name":"archive"}),
    );
    assert_eq!(
        mcp.call(
            "timeline_archive_check",
            json!({"timeline_id":tl,"branch":"archive"})
        ),
        json!({"eligible":true,"reasons":[]})
    );
    assert_eq!(
        mcp.call(
            "timeline_archive_branch",
            json!({"timeline_id":tl,"branch":"archive"})
        ),
        json!({"archived":[]})
    );
    mcp.call("timeline_switch", json!({"timeline_id":tl,"name":"main"}));
    assert_eq!(
        mcp.call(
            "timeline_drop_branch",
            json!({"timeline_id":tl,"name":"旁支"})
        )["dropped"],
        "旁支"
    );
    let failure = mcp.request(
        "tools/call",
        json!({"name":"timeline_fork","arguments":{"timeline_id":tl,"node":"n99999","name":"bad"}}),
    );
    assert_eq!(failure["error"]["code"], -32000);
    assert!(failure["error"]["message"].is_string());
    assert_eq!(
        mcp.called, advertised,
        "every advertised tool exercised in a real process"
    );
}

#[test]
fn interfaces_reject_invalid_inputs_as_json() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("invalid.json");
    cli(&db, &["create"]);
    for args in [
        vec!["append", "--role", "alien", "--text", "x"],
        vec!["budget", "--node", "n0", "--max", "-1"],
        vec!["pin", "--node", "n0/../x"],
        vec!["fork", "--node", "n0", "--name", "a/b"],
        vec!["append", "--role", "user"],
        vec!["does-not-exist"],
    ] {
        assert!(!cli_error(&db, &args).is_empty());
    }
    let mut mcp = Mcp::new(&db);
    let cases = [
        ("timeline_append", json!({"role":"alien","text":"x"})),
        (
            "timeline_append",
            json!({"role":"user","text":"x","tokens":-1}),
        ),
        (
            "timeline_append",
            json!({"role":"user","text":"x","tokens":1.5}),
        ),
        (
            "timeline_append",
            json!({"role":"user","text":"x","tokens":null}),
        ),
        (
            "timeline_context_budget",
            json!({"node":"n0","max_tokens":-1}),
        ),
        (
            "timeline_context_budget",
            json!({"node":"n0","max_tokens":1.5}),
        ),
        (
            "timeline_summarize",
            json!({"from":"n0","to":"n0","summary":"x","derived_from":[1]}),
        ),
        (
            "timeline_summarize",
            json!({"from":"n0","to":"n0","summary":"x","derived_from":"n1"}),
        ),
        (
            "timeline_summarize",
            json!({"from":"n0","to":"n0","summary":"x","supersede":3}),
        ),
        ("timeline_fork", json!({"node":"n0","name":""})),
        ("timeline_fork", json!({"node":"n0","name":"a/b"})),
        ("timeline_node", json!({"node":"bad"})),
        ("timeline_node", json!({"node":"n0","unexpected":true})),
        ("timeline_pin", json!({"node":"n0","idempotency_key":3})),
    ];
    for (name, mut arguments) in cases {
        arguments["timeline_id"] = json!("tl0");
        let response = mcp.request("tools/call", json!({"name":name,"arguments":arguments}));
        assert_eq!(response["error"]["code"], -32000, "{response}");
    }
    assert_eq!(
        mcp.request(
            "tools/call",
            json!({"name":"timeline_branches","arguments":{}})
        )["error"]["code"],
        -32000
    );
    assert_eq!(mcp.raw("this is not JSON")["error"]["code"], -32700);
    assert_eq!(mcp.raw("{}")["error"]["code"], -32600);
    assert_eq!(
        mcp.request("unknown-method", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        mcp.request(
            "tools/call",
            json!({"name":"timeline_append","arguments":[]})
        )["error"]["code"],
        -32602
    );
    assert_eq!(
        mcp.request("ping", json!({}))["result"],
        json!({}),
        "server survives malformed requests"
    );
}

#[test]
fn cli_and_mcp_idempotency_survive_process_restart() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("idempotent.json");
    let first_create = cli(&db, &["--idempotency-key", "create-1", "create"]);
    assert_eq!(
        cli(&db, &["--idempotency-key", "create-1", "create"]),
        first_create
    );
    let append = [
        "--idempotency-key",
        "append-1",
        "append",
        "--role",
        "user",
        "--text",
        "once",
    ];
    let first = cli(&db, &append);
    assert_eq!(cli(&db, &append), first);
    let one = first["node"].as_str().unwrap();
    let summary = [
        "--idempotency-key",
        "summary-1",
        "summarize",
        "--from",
        one,
        "--to",
        one,
        "--summary",
        "saved once",
    ];
    let original = cli(&db, &summary);
    assert_eq!(cli(&db, &summary), original);
    let mut mcp = Mcp::new(&db);
    let params = json!({"name":"timeline_append","idempotency_key":"mcp-append","arguments":{"timeline_id":"tl0","role":"assistant","text":"only once"}});
    let once = mcp.request("tools/call", params.clone())["result"].clone();
    assert!(!once.is_null());
    drop(mcp);
    let mut restarted = Mcp::new(&db);
    assert_eq!(restarted.request("tools/call", params)["result"], once);
    let append_id: Value =
        serde_json::from_str(once["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        restarted
            .call(
                "timeline_context",
                json!({"timeline_id":"tl0","node":append_id["node"]})
            )
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mismatch = restarted.request("tools/call", json!({"name":"timeline_pin","idempotency_key":"outer","arguments":{"timeline_id":"tl0","node":one,"idempotency_key":"inner"}}));
    assert_eq!(mismatch["error"]["code"], -32602);
    drop(restarted);
    let stored: Value = serde_json::from_slice(&std::fs::read(&db).unwrap()).unwrap();
    assert_eq!(stored["timelines"].as_object().unwrap().len(), 1);
    assert_eq!(
        stored["timelines"]["tl0"]["nodes"]
            .as_object()
            .unwrap()
            .len(),
        4,
        "root + two messages + one summary only"
    );
}

#[test]
fn cli_defaults_to_earliest_timeline_and_honors_explicit_selection() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("selection.json");
    for index in 0..12 {
        assert_eq!(cli(&db, &["create"])["timeline_id"], format!("tl{index}"));
    }
    let first = cli(&db, &["append", "--role", "user", "--text", "earliest"]);
    assert_eq!(
        cli(
            &db,
            &[
                "--timeline",
                "tl0",
                "node",
                "--node",
                first["node"].as_str().unwrap()
            ]
        )["text"],
        "earliest"
    );
    let selected = cli(
        &db,
        &[
            "append",
            "--timeline",
            "tl10",
            "--role",
            "user",
            "--text",
            "explicit",
        ],
    );
    assert_eq!(
        cli(
            &db,
            &[
                "--timeline",
                "tl10",
                "node",
                "--node",
                selected["node"].as_str().unwrap()
            ]
        )["text"],
        "explicit"
    );
    assert!(!cli_error(&db, &["node", "--node", selected["node"].as_str().unwrap()]).is_empty());
}

#[test]
fn cli_and_mcp_db_path_precedence() {
    let temp = tempfile::tempdir().unwrap();
    for binary in [env!("CARGO_BIN_EXE_tl"), env!("CARGO_BIN_EXE_tl-mcp")] {
        let home = temp.path().join(
            if binary.ends_with("tl-mcp.exe") || binary.ends_with("tl-mcp") {
                "mcp"
            } else {
                "cli"
            },
        );
        std::fs::create_dir(&home).unwrap();
        let explicit = home.join("explicit.json");
        let environment = home.join("environment.json");
        let default = home.join(".timeline.json");
        let is_mcp = binary == env!("CARGO_BIN_EXE_tl-mcp");
        for mode in ["explicit", "environment", "default"] {
            let mut command = Command::new(binary);
            command.env("USERPROFILE", &home).env("HOME", &home);
            if mode == "default" {
                command.env_remove("TIMELINE_DB");
            } else {
                command.env("TIMELINE_DB", &environment);
            }
            if mode == "explicit" {
                command.arg("--db").arg(&explicit);
            }
            if is_mcp {
                let mut mcp = Mcp::from_command(&mut command);
                assert_eq!(mcp.call("timeline_create", json!({}))["timeline_id"], "tl0");
            } else {
                let output = command.arg("create").output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    serde_json::from_slice::<Value>(&output.stdout).unwrap()["timeline_id"],
                    "tl0"
                );
            }
            match mode {
                "explicit" => {
                    assert!(explicit.exists());
                    assert!(!environment.exists());
                    assert!(!default.exists());
                }
                "environment" => {
                    assert!(environment.exists());
                    assert!(!default.exists());
                }
                "default" => assert!(default.exists()),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn mcp_corrupt_storage_error_does_not_leak_path() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("private-conversation-file.json");
    std::fs::write(&db, b"{broken json").unwrap();
    let mut mcp = Mcp::new(&db);
    let response = mcp.request(
        "tools/call",
        json!({"name":"timeline_create","arguments":{}}),
    );
    assert_eq!(response["error"]["code"], -32000);
    let reason = response["error"]["message"].as_str().unwrap();
    assert!(!reason.contains("private-conversation-file"));
    assert!(reason.contains(".bak"));
    assert_eq!(std::fs::read(&db).unwrap(), b"{broken json");
}

#[test]
fn mcp_bounded_frames_invalid_utf8_and_text_limit_recover() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("bounded-frames.json");
    let mut mcp = Mcp::new(&db);
    assert_eq!(mcp.raw_bytes(&[0xff, 0xfe, b'{'])["error"]["code"], -32700);
    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));
    let too_large = vec![b'x'; MAX_MCP_REQUEST_BYTES + 1];
    let rejected = mcp.raw_bytes(&too_large);
    assert_eq!(rejected["error"]["code"], -32600);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("8 MiB")
    );
    assert_eq!(
        mcp.request("ping", json!({}))["result"],
        json!({}),
        "oversized frames are drained up to the newline"
    );
    mcp.call("timeline_create", json!({}));
    let before = std::fs::read(&db).unwrap();
    let response = mcp.request("tools/call", json!({"name":"timeline_append","arguments":{"timeline_id":"tl0","role":"user","text":"x".repeat(MAX_TEXT_BYTES + 1)}}));
    assert_eq!(response["error"]["code"], -32000);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("1 MiB")
    );
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn mcp_validates_request_ids_and_negotiates_supported_protocols() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("protocol.json");
    let mut mcp = Mcp::new(&db);
    for id in [Value::Null, json!(1.5), json!(true), json!([])] {
        let response = mcp.raw(&json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"timeline_create","arguments":{}}}).to_string());
        assert_eq!(response["error"]["code"], -32600);
    }
    assert!(
        !db.exists(),
        "invalid request IDs must never invoke a mutation"
    );
    for protocol in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
        let initialize = mcp.request("initialize", json!({"protocolVersion":protocol,"capabilities":{},"clientInfo":{"name":"acceptance","version":"1"}}));
        assert_eq!(initialize["result"]["protocolVersion"], protocol);
        let response = mcp.request(
            "tools/call",
            json!({"name":"timeline_create","arguments":{}}),
        );
        assert_eq!(
            response["result"].get("structuredContent").is_some(),
            protocol >= "2025-06-18",
            "structuredContent is a negotiated feature"
        );
    }
    let future = mcp.request("initialize", json!({"protocolVersion":"2099-01-01","capabilities":{},"clientInfo":{"name":"acceptance","version":"1"}}));
    assert_eq!(
        future["result"]["protocolVersion"], "2025-11-25",
        "return a supported protocol for negotiation"
    );
    for params in [
        json!({}),
        json!({"protocolVersion":3,"capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
        json!({"protocolVersion":"2024-11-05","capabilities":[],"clientInfo":{"name":"test","version":"1"}}),
    ] {
        assert_eq!(mcp.request("initialize", params)["error"]["code"], -32602);
    }
}

#[test]
fn readonly_tools_accept_keys_without_caching_or_writing() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("read-key.json");
    let mut mcp = Mcp::new(&db);
    mcp.call("timeline_create", json!({}));
    let one = mcp.call(
        "timeline_append",
        json!({"timeline_id":"tl0","role":"user","text":"first","idempotency_key":"shared"}),
    )["node"]
        .clone();
    let before_query = std::fs::read(&db).unwrap();
    let query = json!({"timeline_id":"tl0","idempotency_key":"shared"});
    let original = mcp.call("timeline_branches", query.clone());
    assert_eq!(original[0]["head"], one);
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before_query,
        "queries do not persist a deduplication entry or write a snapshot"
    );
    let two = mcp.call(
        "timeline_append",
        json!({"timeline_id":"tl0","role":"assistant","text":"second"}),
    )["node"]
        .clone();
    let current = mcp.call("timeline_branches", query);
    assert_eq!(
        current[0]["head"], two,
        "read keys never return stale mutation results"
    );
}

#[test]
fn mcp_legacy_batch_compatibility_is_serial_and_ignores_notifications() {
    let temp = tempfile::tempdir().unwrap();
    let mut mcp = Mcp::new(&temp.path().join("legacy-batch.json"));
    let initialize = mcp.request("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"batch-test","version":"1"}}));
    assert_eq!(initialize["result"]["protocolVersion"], "2025-03-26");
    let response = mcp.raw(&json!([
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":"create","method":"tools/call","params":{"name":"timeline_create","arguments":{}}},
        {"jsonrpc":"2.0","id":"append","method":"tools/call","params":{"name":"timeline_append","arguments":{"timeline_id":"tl0","role":"user","text":"after create"}}},
        7
    ]).to_string());
    let responses = response.as_array().unwrap();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], "create");
    assert_eq!(responses[1]["id"], "append");
    assert!(
        responses[1].get("error").is_none(),
        "batch calls execute in order"
    );
    assert_eq!(responses[2]["error"]["code"], -32600);
    writeln!(
        mcp.stdin.as_mut().unwrap(),
        "{}",
        json!([
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"none"}}
        ])
    )
    .unwrap();
    assert_eq!(
        mcp.request("ping", json!({}))["result"],
        json!({}),
        "notification-only batch has no response"
    );
    assert_eq!(mcp.raw("[]")["error"]["code"], -32600);
    mcp.request("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"batch-test","version":"1"}}));
    assert_eq!(
        mcp.raw(r#"[{"jsonrpc":"2.0","id":"batch","method":"ping"}]"#)["error"]["code"],
        -32600
    );
}
