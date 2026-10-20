use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use timeline::service::call_storage;
use timeline::storage::{Storage, resolve_db_path};
use timeline::{Database, Error, Result, ascii, drawio};

#[derive(Parser)]
#[command(
    name = "tl",
    version = env!("CARGO_PKG_VERSION"),
    about = "Persistent, reversible conversation timelines"
)]
struct Cli {
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    timeline: Option<String>,
    #[arg(long, global = true)]
    idempotency_key: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Create,
    Append {
        #[arg(long)]
        role: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        tokens: Option<u64>,
        #[arg(long)]
        intent_tag: Option<String>,
        #[arg(long)]
        intent_note: Option<String>,
    },
    Fork {
        #[arg(long)]
        node: String,
        #[arg(long)]
        name: String,
    },
    Switch {
        #[arg(long)]
        name: String,
    },
    DropBranch {
        #[arg(long)]
        name: String,
    },
    Context {
        #[arg(long)]
        node: String,
    },
    Budget {
        #[arg(long)]
        node: String,
        #[arg(long)]
        max: u64,
    },
    Pin {
        #[arg(long)]
        node: String,
    },
    Unpin {
        #[arg(long)]
        node: String,
    },
    Summarize {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        tokens: Option<u64>,
        #[arg(long, value_delimiter = ',')]
        derived_from: Vec<String>,
        #[arg(long)]
        supersede: Option<String>,
        #[arg(long)]
        intent_tag: Option<String>,
        #[arg(long)]
        intent_note: Option<String>,
    },
    Branches,
    Node {
        #[arg(long)]
        node: String,
    },
    ArchiveCheck {
        #[arg(long)]
        name: String,
    },
    ArchiveBranch {
        #[arg(long)]
        name: String,
    },
    /// Export a read-only diagram. ASCII defaults to stdout; existing files are never overwritten.
    Export {
        /// Export format (defaults to drawio).
        #[arg(long, value_enum)]
        format: Option<ExportFormat>,
        /// Destination file. Required for drawio; omit for ASCII terminal output.
        #[arg(
            long,
            required_unless_present = "format",
            required_if_eq("format", "drawio")
        )]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    Drawio,
    Ascii,
}

enum CommandOutput {
    Json(Value),
    Text(String),
}

fn default_timeline(db: &Database) -> Result<String> {
    db.timelines
        .keys()
        .min_by_key(|id| {
            id.strip_prefix("tl")
                .and_then(|number| number.parse::<u64>().ok())
                .unwrap_or(u64::MAX)
        })
        .cloned()
        .ok_or_else(|| Error::new("no timeline exists; run tl create first".into()))
}

fn export(
    storage: &Storage,
    requested: Option<&str>,
    format: ExportFormat,
    output: Option<&Path>,
) -> Result<CommandOutput> {
    // One validated, consistent read. Do not go through mutating budget/MCP calls.
    let db = storage.read()?;
    let id = requested
        .map(str::to_owned)
        .map_or_else(|| default_timeline(&db), Ok)?;
    let timeline = db.timeline(&id)?;
    let (name, contents, nodes, edges) = match format {
        ExportFormat::Drawio => {
            let diagram = drawio::render(timeline)?;
            ("drawio", diagram.xml, diagram.nodes, diagram.edges)
        }
        ExportFormat::Ascii => {
            let diagram = ascii::render(timeline)?;
            ("ascii", diagram.text, diagram.nodes, diagram.edges)
        }
    };
    let Some(output) = output else {
        return Ok(CommandOutput::Text(contents));
    };
    save_export(output, contents.as_bytes())?;
    Ok(CommandOutput::Json(json!({
        "format":name,"timeline_id":id,"output":output.to_string_lossy(),"nodes":nodes,"edges":edges
    })))
}

fn save_export(output: &Path, contents: &[u8]) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::Builder::new()
        .prefix(".timeline-export-")
        .tempfile_in(parent)
        .map_err(|_| Error::new("cannot create export file; output directory must exist".into()))?;
    file.write_all(contents)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| Error::new("cannot write export file".into()))?;
    // Never clobber a user's diagram, database, backup, symlink or concurrent export.
    file.persist_noclobber(output).map_err(|error| {
        Error::new(
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                "output already exists; choose a new path to preserve manual edits"
            } else {
                "cannot save export file; choose a new writable output path"
            }
            .into(),
        )
    })?;
    Ok(())
}

fn intent_value(tag: Option<String>, note: Option<String>) -> Result<Value> {
    match (tag, note) {
        (Some(tag), note) => {
            let mut object = serde_json::Map::new();
            object.insert("tag".into(), json!(tag));
            if let Some(note) = note {
                object.insert("note".into(), json!(note));
            }
            Ok(Value::Object(object))
        }
        (None, Some(_)) => Err(Error::new("--intent-note requires --intent-tag".into())),
        (None, None) => Ok(Value::Null),
    }
}

fn run(cli: Cli) -> Result<CommandOutput> {
    let storage = Storage::new(resolve_db_path(cli.db)?);
    let (tool, mut args) = match cli.command {
        Command::Create => ("timeline_create", json!({})),
        Command::Append {
            role,
            text,
            tokens,
            intent_tag,
            intent_note,
        } => (
            "timeline_append",
            json!({"role":role,"text":text,"tokens":tokens,"intent":intent_value(intent_tag, intent_note)?}),
        ),
        Command::Fork { node, name } => ("timeline_fork", json!({"node":node,"name":name})),
        Command::Switch { name } => ("timeline_switch", json!({"name":name})),
        Command::DropBranch { name } => ("timeline_drop_branch", json!({"name":name})),
        Command::Context { node } => ("timeline_context", json!({"node":node})),
        Command::Budget { node, max } => (
            "timeline_context_budget",
            json!({"node":node,"max_tokens":max}),
        ),
        Command::Pin { node } => ("timeline_pin", json!({"node":node})),
        Command::Unpin { node } => ("timeline_unpin", json!({"node":node})),
        Command::Summarize {
            from,
            to,
            summary,
            tokens,
            derived_from,
            supersede,
            intent_tag,
            intent_note,
        } => (
            "timeline_summarize",
            json!({"from":from,"to":to,"summary":summary,"tokens":tokens,"derived_from":derived_from,"supersede":supersede,"intent":intent_value(intent_tag, intent_note)?}),
        ),
        Command::Branches => ("timeline_branches", json!({})),
        Command::Node { node } => ("timeline_node", json!({"node":node})),
        Command::ArchiveCheck { name } => ("timeline_archive_check", json!({"branch":name})),
        Command::ArchiveBranch { name } => ("timeline_archive_branch", json!({"branch":name})),
        Command::Export { format, output } => {
            if cli.idempotency_key.is_some() {
                return Err(Error::new(
                    "export is read-only and does not accept idempotency_key".into(),
                ));
            }
            return export(
                &storage,
                cli.timeline.as_deref(),
                format.unwrap_or(ExportFormat::Drawio),
                output.as_deref(),
            );
        }
    };
    args.as_object_mut()
        .expect("object")
        .retain(|_, value| !value.is_null());
    if tool != "timeline_create" {
        let id = match cli.timeline {
            Some(id) => id,
            None => default_timeline(&storage.read()?)?,
        };
        args["timeline_id"] = json!(id);
    }
    if let Some(key) = cli.idempotency_key {
        args["idempotency_key"] = json!(key);
    }
    call_storage(&storage, tool, &args).map(CommandOutput::Json)
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{error}");
                return;
            }
            eprintln!("{}", json!({"error":error.to_string()}));
            std::process::exit(2);
        }
    };
    match run(cli) {
        Ok(output) => {
            let contents = match output {
                CommandOutput::Json(value) => format!("{value}\n"),
                CommandOutput::Text(text) => text,
            };
            if let Err(error) = io::stdout().lock().write_all(contents.as_bytes()) {
                // Pagers and `head` may close their pipe before the diagram ends.
                if error.kind() != io::ErrorKind::BrokenPipe {
                    eprintln!("{}", json!({"error":"cannot write stdout"}));
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            let mut response = json!({"error":error.to_string()});
            if let Some(data) = error.data {
                response["data"] = data;
            }
            eprintln!("{response}");
            std::process::exit(1);
        }
    }
}
