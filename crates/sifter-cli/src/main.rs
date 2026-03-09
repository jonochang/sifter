use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use sifter_core::config::{ConfigStore, matching_contexts};

#[derive(Debug, Parser)]
#[command(name = "sifter")]
#[command(author, version, about = "Local-first search for code and docs")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Collection(CollectionCommand),
    Context(ContextCommand),
}

#[derive(Debug, Args)]
struct CollectionCommand {
    #[command(subcommand)]
    command: CollectionSubcommand,
}

#[derive(Debug, Subcommand)]
enum CollectionSubcommand {
    Add(CollectionAdd),
    List(OutputArgs),
}

#[derive(Debug, Args)]
struct CollectionAdd {
    path: PathBuf,
    #[arg(long)]
    name: String,
    #[arg(long = "mask")]
    pattern: Option<String>,
}

#[derive(Debug, Args)]
struct ContextCommand {
    #[command(subcommand)]
    command: ContextSubcommand,
}

#[derive(Debug, Subcommand)]
enum ContextSubcommand {
    Add(ContextAdd),
    List(OutputArgs),
    Check(ContextCheck),
    Rm(ContextRemove),
}

#[derive(Debug, Args)]
struct ContextAdd {
    scope: String,
    value: String,
}

#[derive(Debug, Args)]
struct ContextCheck {
    scope: String,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct ContextRemove {
    scope: String,
}

#[derive(Debug, Args, Clone, Default)]
struct OutputArgs {
    #[arg(long)]
    json: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        execute(command)?;
    }
    Ok(())
}

fn execute(command: Command) -> Result<()> {
    let store = ConfigStore::new("default")?;
    match command {
        Command::Collection(command) => match command.command {
            CollectionSubcommand::Add(args) => {
                let config = store.add_collection(&args.name, args.path, args.pattern)?;
                print_json(&json!({
                    "collection": {
                        "name": args.name,
                        "path": config
                            .collections
                            .get(&args.name)
                            .ok_or_else(|| anyhow!("collection was not stored"))?
                            .path,
                    }
                }));
            }
            CollectionSubcommand::List(output) => {
                let config = store.load()?;
                let collections = config
                    .collections
                    .iter()
                    .map(|(name, collection)| {
                        json!({
                            "name": name,
                            "path": collection.path,
                            "pattern": collection.pattern,
                            "includeByDefault": collection.include_by_default,
                        })
                    })
                    .collect::<Vec<_>>();
                print_output(
                    output.json,
                    &json!({
                        "collections": collections,
                    }),
                )?;
            }
        },
        Command::Context(command) => match command.command {
            ContextSubcommand::Add(args) => {
                store.add_context(&args.scope, &args.value)?;
                print_json(&json!({
                    "context": {
                        "scope": args.scope,
                        "value": args.value,
                    }
                }));
            }
            ContextSubcommand::List(output) => {
                let config = store.load()?;
                let contexts = config
                    .contexts
                    .iter()
                    .map(|(scope, value)| {
                        json!({
                            "scope": scope,
                            "value": value,
                        })
                    })
                    .collect::<Vec<_>>();
                print_output(output.json, &json!({ "contexts": contexts }))?;
            }
            ContextSubcommand::Check(args) => {
                let config = store.load()?;
                let matches = matching_contexts(&config, &args.scope)
                    .into_iter()
                    .map(|item| {
                        json!({
                            "scope": item.scope,
                            "value": item.value,
                        })
                    })
                    .collect::<Vec<_>>();
                print_output(args.output.json, &json!({ "matches": matches }))?;
            }
            ContextSubcommand::Rm(args) => {
                store.remove_context(&args.scope)?;
                print_json(&json!({
                    "removed": args.scope,
                }));
            }
        },
    }
    Ok(())
}

fn print_output(force_json: bool, value: &serde_json::Value) -> Result<()> {
    if force_json || !std::io::stdout().is_terminal() {
        print_json(value);
        return Ok(());
    }

    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_json(value: &serde_json::Value) {
    println!("{value}");
}
