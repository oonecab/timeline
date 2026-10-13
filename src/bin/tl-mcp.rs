use clap::Parser;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use timeline::service::{MAX_MCP_REQUEST_BYTES, call_storage, tool_definitions};
use timeline::storage::{Storage, resolve_db_path};

#[derive(Parser)]
#[command(
    name = "tl-mcp",
    version = "1.1.0",
    about = "Timeline MCP server over newline-delimited stdio JSON-RPC"
)]
struct Cli {
    #[arg(long)]
    db: Option<PathBuf>,
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

const SUPPORTED_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

fn handle(
    request: &Value,
    storage: &Storage,
    protocol_version: &mut &'static str,
) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !request.is_object()
        || request.get("jsonrpc") != Some(&json!("2.0"))
        || request.get("method").and_then(Value::as_str).is_none()
        || (request.get("id").is_some() && !(id.is_string() || id.is_i64() || id.is_u64()))
    {
        return Some(error(Value::Null, -32600, "invalid JSON-RPC request"));
    }
    let method = request["method"].as_str().expect("validated method");
    let notification = request.get("id").is_none();
    // Notifications do not receive responses or invoke mutating tools.
    if notification {
        return None;
    }
    let empty = json!({});
    let params = request.get("params").unwrap_or(&empty);
    if !params.is_object() {
        return Some(error(id, -32602, "params must be an object"));
    }
    let result = match method {
        "initialize" => {
            let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) else {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires a string protocolVersion",
                ));
            };
            if !params.get("capabilities").is_some_and(Value::is_object)
                || params
                    .get("clientInfo")
                    .and_then(|info| info.get("name"))
                    .and_then(Value::as_str)
                    .is_none()
                || params
                    .get("clientInfo")
                    .and_then(|info| info.get("version"))
                    .and_then(Value::as_str)
                    .is_none()
            {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires capabilities and clientInfo with name and version",
                ));
            }
            // Never claim support for an unimplemented future protocol. The client
            // may reconnect using the newest common version after negotiation.
            *protocol_version = SUPPORTED_VERSIONS
                .iter()
                .copied()
                .find(|version| *version == requested)
                .unwrap_or("2025-11-25");
            json!({"protocolVersion":*protocol_version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"timeline","version":"1.1.0"}})
        }
        "ping" | "notifications/initialized" => json!({}),
        "tools/list" => json!({"tools":tool_definitions()}),
        "tools/call" => {
            let Some(tool) = params.get("name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "tools/call requires a tool name"));
            };
            let mut args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !args.is_object() {
                return Some(error(id, -32602, "arguments must be an object"));
            }
            if let Some(key) = params.get("idempotency_key") {
                if !key.is_string() {
                    return Some(error(id, -32602, "idempotency_key must be a string"));
                }
                if args
                    .get("idempotency_key")
                    .is_some_and(|argument_key| argument_key != key)
                {
                    return Some(error(id, -32602, "conflicting idempotency_key values"));
                }
                args["idempotency_key"] = key.clone();
            }
            match call_storage(storage, tool, &args) {
                Ok(value) => {
                    let mut result = json!({"content":[{"type":"text","text":value.to_string()}],"isError":false});
                    if *protocol_version >= "2025-06-18" && value.is_object() {
                        result["structuredContent"] = value;
                    }
                    result
                }
                Err(reason) => return Some(error(id, -32000, &reason.to_string())),
            }
        }
        _ => return Some(error(id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

enum Frame {
    Message(Vec<u8>),
    TooLarge,
}

/// Drain oversized frames without allocating beyond the advertised limit, then
/// resume at the following newline. Invalid UTF-8 is handled as a parse error.
fn read_frame(input: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let mut bytes = Vec::new();
    let mut too_large = false;
    let mut saw_bytes = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if too_large {
                Some(Frame::TooLarge)
            } else if saw_bytes {
                Some(Frame::Message(bytes))
            } else {
                None
            });
        }
        saw_bytes = true;
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.unwrap_or(buffer.len());
        if !too_large {
            if length > MAX_MCP_REQUEST_BYTES - bytes.len() {
                too_large = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(&buffer[..length]);
            }
        }
        input.consume(length + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(if too_large {
                Frame::TooLarge
            } else {
                Frame::Message(bytes)
            }));
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let storage = match resolve_db_path(cli.db) {
        Ok(path) => Storage::new(path),
        Err(reason) => {
            eprintln!("{}", json!({"error":reason.to_string()}));
            std::process::exit(1);
        }
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut input = stdin.lock();
    let mut protocol_version = "2024-11-05";
    while let Ok(Some(frame)) = read_frame(&mut input) {
        let response = match frame {
            Frame::TooLarge => Some(error(
                Value::Null,
                -32600,
                "request exceeds the 8 MiB limit",
            )),
            Frame::Message(bytes) => {
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(Value::Array(requests))
                        if protocol_version <= "2025-03-26" && !requests.is_empty() =>
                    {
                        let responses: Vec<Value> = requests
                            .iter()
                            .filter_map(|request| handle(request, &storage, &mut protocol_version))
                            .collect();
                        (!responses.is_empty()).then_some(Value::Array(responses))
                    }
                    Ok(Value::Array(_)) => Some(error(
                        Value::Null,
                        -32600,
                        "empty batch or batching unsupported by negotiated protocol",
                    )),
                    Ok(request) => handle(&request, &storage, &mut protocol_version),
                    Err(_) => Some(error(Value::Null, -32700, "invalid JSON or UTF-8")),
                }
            }
        };
        if let Some(response) = response {
            if writeln!(output, "{response}")
                .and_then(|_| output.flush())
                .is_err()
            {
                break;
            }
        }
    }
}
