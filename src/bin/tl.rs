use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;
use timeline::service::call_storage;
use timeline::storage::{Storage, resolve_db_path};
use timeline::{Error, Result};

#[derive(Parser)]
#[command(
    name = "tl",
    version = "1.1.0",
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
}

fn run(cli: Cli) -> Result<Value> {
    let storage = Storage::new(resolve_db_path(cli.db)?);
    let (tool, mut args) = match cli.command {
        Command::Create => ("timeline_create", json!({})),
        Command::Append { role, text, tokens } => (
            "timeline_append",
            json!({"role":role,"text":text,"tokens":tokens}),
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
        } => (
            "timeline_summarize",
            json!({"from":from,"to":to,"summary":summary,"tokens":tokens,"derived_from":derived_from,"supersede":supersede}),
        ),
        Command::Branches => ("timeline_branches", json!({})),
        Command::Node { node } => ("timeline_node", json!({"node":node})),
        Command::ArchiveCheck { name } => ("timeline_archive_check", json!({"branch":name})),
        Command::ArchiveBranch { name } => ("timeline_archive_branch", json!({"branch":name})),
    };
    args.as_object_mut()
        .expect("object")
        .retain(|_, value| !value.is_null());
    if tool != "timeline_create" {
        let id = match cli.timeline {
            Some(id) => id,
            None => storage
                .read()?
                .timelines
                .keys()
                .min_by_key(|id| {
                    id.strip_prefix("tl")
                        .and_then(|number| number.parse::<u64>().ok())
                        .unwrap_or(u64::MAX)
                })
                .cloned()
                .ok_or_else(|| Error("no timeline exists; run tl create first".into()))?,
        };
        args["timeline_id"] = json!(id);
    }
    if let Some(key) = cli.idempotency_key {
        args["idempotency_key"] = json!(key);
    }
    call_storage(&storage, tool, &args)
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
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", json!({"error":error.to_string()}));
            std::process::exit(1);
        }
    }
}
