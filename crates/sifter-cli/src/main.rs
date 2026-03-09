use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use sifter_core::config::{ConfigStore, cache_file_path, matching_contexts};
use sifter_store::index::Store;

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
    Update(OutputArgs),
    Status(OutputArgs),
    Search(SearchCommand),
    Symbol(SearchCommand),
    Related(RelatedCommand),
    Get(GetCommand),
    MultiGet(MultiGetCommand),
    Embed(VectorCommand),
    Vsearch(VectorCommand),
    Query(VectorCommand),
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

#[derive(Debug, Args)]
struct SearchCommand {
    query: String,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct GetCommand {
    reference: String,
}

#[derive(Debug, Args)]
struct RelatedCommand {
    reference: String,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct MultiGetCommand {
    references: String,
}

#[derive(Debug, Args)]
struct VectorCommand {
    query: Option<String>,
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
        Command::Update(output) => {
            let config = store.load()?;
            let db_path = cache_file_path("default")?;
            let mut index = Store::open(&db_path)?;
            let indexed_files = index.rebuild(&config)?;
            print_output(output.json, &json!({ "indexed_files": indexed_files }))?;
        }
        Command::Status(output) => {
            let config = store.load()?;
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            print_output(output.json, &serde_json::to_value(index.status(&config)?)?)?;
        }
        Command::Search(args) => {
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            let results = index.search(&args.query)?;
            print_output(
                args.output.json,
                &json!({
                    "results": results,
                }),
            )?;
        }
        Command::Symbol(args) => {
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            let results = index.symbol(&args.query)?;
            print_output(args.output.json, &json!({ "results": results }))?;
        }
        Command::Related(args) => {
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            let results = index.related(&args.reference)?;
            print_output(args.output.json, &json!({ "results": results }))?;
        }
        Command::Get(args) => {
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            let file = index
                .get(&args.reference)?
                .ok_or_else(|| anyhow!("reference not found: {}", args.reference))?;
            print_json(&json!({
                "docid": file.docid,
                "file": file.path,
                "virtual_path": file.virtual_path,
                "kind": file.kind,
                "title": file.title,
                "language": file.language,
                "content": file.content,
                "line_start": file.line_start,
                "line_end": file.line_end,
            }));
        }
        Command::MultiGet(args) => {
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            let results = index.multi_get(&args.references)?;
            print_json(&json!({
                "results": results,
            }));
        }
        Command::Embed(args) => return vector_pending("embed", args.query.as_deref()),
        Command::Vsearch(args) => return vector_pending("vsearch", args.query.as_deref()),
        Command::Query(args) => return vector_pending("query", args.query.as_deref()),
    }
    Ok(())
}

fn vector_pending(command: &str, _query: Option<&str>) -> Result<()> {
    let payload = json!({
        "error": "vector_runtime_pending",
        "command": command,
        "message": "vector runtime is not implemented in this build yet",
    });
    print_json(&payload);
    std::process::exit(2);
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
