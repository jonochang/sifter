use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{ArgGroup, Args, Parser, Subcommand};
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
    Config(ConfigCommand),
    Index(IndexCommand),
    Search(SearchCommand),
    Show(ShowCommand),
}

#[derive(Debug, Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
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

#[derive(Debug, Args)]
struct IndexCommand {
    #[command(subcommand)]
    command: IndexSubcommand,
}

#[derive(Debug, Subcommand)]
enum IndexSubcommand {
    Update(OutputArgs),
    Status(OutputArgs),
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("search_mode")
        .args(["semantic", "hybrid", "symbol", "related"])
        .multiple(false)
))]
struct SearchCommand {
    query: Option<String>,
    #[arg(long)]
    semantic: bool,
    #[arg(long)]
    hybrid: bool,
    #[arg(long)]
    symbol: Option<String>,
    #[arg(long)]
    related: Option<String>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct ShowCommand {
    references: Vec<String>,
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
    let config_store = ConfigStore::new("default")?;
    match command {
        Command::Config(command) => execute_config(command, &config_store)?,
        Command::Index(command) => execute_index(command, &config_store)?,
        Command::Search(command) => execute_search(command)?,
        Command::Show(command) => execute_show(command)?,
    }
    Ok(())
}

fn execute_config(command: ConfigCommand, config_store: &ConfigStore) -> Result<()> {
    match command.command {
        ConfigSubcommand::Collection(command) => match command.command {
            CollectionSubcommand::Add(args) => {
                let config = config_store.add_collection(&args.name, args.path, args.pattern)?;
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
                let config = config_store.load()?;
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
                print_output(output.json, &json!({ "collections": collections }))?;
            }
        },
        ConfigSubcommand::Context(command) => match command.command {
            ContextSubcommand::Add(args) => {
                config_store.add_context(&args.scope, &args.value)?;
                print_json(&json!({
                    "context": {
                        "scope": args.scope,
                        "value": args.value,
                    }
                }));
            }
            ContextSubcommand::List(output) => {
                let config = config_store.load()?;
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
                let config = config_store.load()?;
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
                config_store.remove_context(&args.scope)?;
                print_json(&json!({
                    "removed": args.scope,
                }));
            }
        },
    }
    Ok(())
}

fn execute_index(command: IndexCommand, config_store: &ConfigStore) -> Result<()> {
    match command.command {
        IndexSubcommand::Update(output) => {
            let config = config_store.load()?;
            let db_path = cache_file_path("default")?;
            let mut index = Store::open(&db_path)?;
            let indexed_files = index.rebuild(&config)?;
            print_output(output.json, &json!({ "indexed_files": indexed_files }))?;
        }
        IndexSubcommand::Status(output) => {
            let config = config_store.load()?;
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            print_output(output.json, &serde_json::to_value(index.status(&config)?)?)?;
        }
    }
    Ok(())
}

fn execute_search(command: SearchCommand) -> Result<()> {
    let db_path = cache_file_path("default")?;
    let index = Store::open(&db_path)?;

    if let Some(symbol) = &command.symbol {
        let results = index.symbol(symbol)?;
        return print_output(command.output.json, &json!({ "results": results }));
    }

    if let Some(reference) = &command.related {
        let results = index.related(reference)?;
        return print_output(command.output.json, &json!({ "results": results }));
    }

    let query = command
        .query
        .as_deref()
        .ok_or_else(|| anyhow!("search requires a query"))?;

    if command.semantic {
        return vector_pending("search --semantic", Some(query));
    }

    if command.hybrid {
        return vector_pending("search --hybrid", Some(query));
    }

    let results = index.search(query)?;
    print_output(command.output.json, &json!({ "results": results }))
}

fn execute_show(command: ShowCommand) -> Result<()> {
    if command.references.is_empty() {
        return Err(anyhow!("show requires at least one reference"));
    }

    let db_path = cache_file_path("default")?;
    let index = Store::open(&db_path)?;

    if command.references.len() == 1 {
        let file = index
            .get(&command.references[0])?
            .ok_or_else(|| anyhow!("reference not found: {}", command.references[0]))?;
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
    } else {
        let references = command.references.join(",");
        let results = index.multi_get(&references)?;
        print_json(&json!({
            "results": results,
        }));
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
