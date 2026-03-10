use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{ArgGroup, Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use sifter_core::config::{ConfigStore, cache_file_path, matching_contexts};
use sifter_store::index::{IndexedFile, LineSlice, SearchKind, SearchOptions, Store, SymbolMode};

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
    Show(CollectionShow),
    Remove(CollectionRemove),
    Rename(CollectionRename),
    Include(CollectionInclude),
    Exclude(CollectionExclude),
    UpdateCmd(CollectionUpdateCommand),
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
struct CollectionRemove {
    name: String,
}

#[derive(Debug, Args)]
struct CollectionShow {
    name: String,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct CollectionRename {
    from: String,
    to: String,
}

#[derive(Debug, Args)]
struct CollectionInclude {
    name: String,
}

#[derive(Debug, Args)]
struct CollectionExclude {
    name: String,
}

#[derive(Debug, Args)]
struct CollectionUpdateCommand {
    name: String,
    command: Option<String>,
}

#[derive(Debug, Args)]
struct ContextCommand {
    #[command(subcommand)]
    command: ContextSubcommand,
}

#[derive(Debug, Subcommand)]
enum ContextSubcommand {
    Add(ContextAdd),
    Global(ContextGlobal),
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
struct ContextGlobal {
    value: Option<String>,
    #[arg(long)]
    clear: bool,
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
#[command(group(
    ArgGroup::new("kind_filter")
        .args(["docs", "code"])
        .multiple(false)
))]
#[command(group(
    ArgGroup::new("symbol_mode")
        .args(["defs", "refs"])
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
    defs: bool,
    #[arg(long)]
    refs: bool,
    #[arg(long)]
    related: Option<String>,
    #[arg(long)]
    docs: bool,
    #[arg(long)]
    code: bool,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct ShowCommand {
    references: Vec<String>,
    #[arg(short = 'l', long = "max-lines")]
    max_lines: Option<usize>,
    #[arg(long = "line-numbers")]
    line_numbers: bool,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args, Clone, Default)]
#[command(group(
    ArgGroup::new("format")
        .args(["json", "csv", "md", "xml", "files"])
        .multiple(false)
))]
struct OutputArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    csv: bool,
    #[arg(long)]
    md: bool,
    #[arg(long)]
    xml: bool,
    #[arg(long)]
    files: bool,
    #[arg(long)]
    full: bool,
}

#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Json,
    Csv,
    Markdown,
    Xml,
    Files,
    Human,
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
                print_serialized(output.format(), &collections, "collections")?;
            }
            CollectionSubcommand::Show(args) => {
                let collection = config_store.collection(&args.name)?;
                print_value(
                    args.output.format(),
                    &json!({
                        "name": args.name,
                        "path": collection.path,
                        "pattern": collection.pattern,
                        "ignore": collection.ignore,
                        "context": collection.context,
                        "update": collection.update,
                        "includeByDefault": collection.include_by_default,
                    }),
                )?;
            }
            CollectionSubcommand::Remove(args) => {
                config_store.remove_collection(&args.name)?;
                print_json(&json!({
                    "removed": args.name,
                }));
            }
            CollectionSubcommand::Rename(args) => {
                config_store.rename_collection(&args.from, &args.to)?;
                print_json(&json!({
                    "renamed": {
                        "from": args.from,
                        "to": args.to,
                    }
                }));
            }
            CollectionSubcommand::Include(args) => {
                config_store.set_collection_included(&args.name, true)?;
                print_json(&json!({
                    "collection": args.name,
                    "includeByDefault": true,
                }));
            }
            CollectionSubcommand::Exclude(args) => {
                config_store.set_collection_included(&args.name, false)?;
                print_json(&json!({
                    "collection": args.name,
                    "includeByDefault": false,
                }));
            }
            CollectionSubcommand::UpdateCmd(args) => {
                config_store.set_collection_update_command(&args.name, args.command.clone())?;
                print_json(&json!({
                    "collection": args.name,
                    "update": args.command,
                }));
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
            ContextSubcommand::Global(args) => {
                let value = if args.clear { None } else { args.value.clone() };
                let config = config_store.set_global_context(value)?;
                print_json(&json!({
                    "global_context": config.global_context,
                }));
            }
            ContextSubcommand::List(output) => {
                let config = config_store.load()?;
                let contexts = config
                    .contexts
                    .iter()
                    .map(|(scope, value)| json!({ "scope": scope, "value": value }))
                    .collect::<Vec<_>>();
                print_serialized(output.format(), &contexts, "contexts")?;
            }
            ContextSubcommand::Check(args) => {
                let config = config_store.load()?;
                let matches = matching_contexts(&config, &args.scope)
                    .into_iter()
                    .map(|item| json!({ "scope": item.scope, "value": item.value }))
                    .collect::<Vec<_>>();
                print_serialized(args.output.format(), &matches, "matches")?;
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
            print_value(output.format(), &json!({ "indexed_files": indexed_files }))?;
        }
        IndexSubcommand::Status(output) => {
            let config = config_store.load()?;
            let db_path = cache_file_path("default")?;
            let index = Store::open(&db_path)?;
            print_value(
                output.format(),
                &serde_json::to_value(index.status(&config)?)?,
            )?;
        }
    }
    Ok(())
}

fn execute_search(command: SearchCommand) -> Result<()> {
    let db_path = cache_file_path("default")?;
    let index = Store::open(&db_path)?;

    if let Some(symbol) = &command.symbol {
        let mode = if command.refs {
            SymbolMode::References
        } else {
            SymbolMode::Definitions
        };
        let results = index.symbol(symbol, mode)?;
        return print_serialized(command.output.format(), &results, "results");
    }

    if let Some(reference) = &command.related {
        let results = index.related(reference)?;
        return print_serialized(command.output.format(), &results, "results");
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

    let kind = if command.docs {
        Some(SearchKind::Doc)
    } else if command.code {
        Some(SearchKind::Code)
    } else {
        None
    };
    let results = index.search(
        query,
        &SearchOptions {
            kind,
            include_full_content: command.output.full,
        },
    )?;

    if matches!(command.output.format(), OutputFormat::Files) {
        let files = results
            .iter()
            .map(|item| item.file.clone())
            .collect::<Vec<_>>();
        return print_serialized(command.output.format(), &files, "files");
    }

    print_serialized(command.output.format(), &results, "results")
}

fn execute_show(command: ShowCommand) -> Result<()> {
    if command.references.is_empty() {
        return Err(anyhow!("show requires at least one reference"));
    }

    let db_path = cache_file_path("default")?;
    let index = Store::open(&db_path)?;
    let slice = if command.max_lines.is_some() || command.line_numbers {
        Some(LineSlice {
            start: 1,
            max_lines: command.max_lines,
            line_numbers: command.line_numbers,
        })
    } else {
        None
    };

    if command.references.len() == 1
        && !command.references[0].contains('*')
        && !command.references[0].contains('?')
    {
        let file = index
            .get(&command.references[0], slice)?
            .ok_or_else(|| anyhow!("reference not found: {}", command.references[0]))?;
        return print_value(command.output.format(), &file_value(&file));
    }

    let results = index.multi_get(&command.references, slice)?;
    let values = results.iter().map(file_value).collect::<Vec<_>>();
    print_serialized(command.output.format(), &values, "results")
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

impl OutputArgs {
    fn format(&self) -> OutputFormat {
        if self.csv {
            OutputFormat::Csv
        } else if self.md {
            OutputFormat::Markdown
        } else if self.xml {
            OutputFormat::Xml
        } else if self.files {
            OutputFormat::Files
        } else if self.json || !std::io::stdout().is_terminal() {
            OutputFormat::Json
        } else {
            OutputFormat::Human
        }
    }
}

fn print_serialized<T>(format: OutputFormat, items: &[T], root: &str) -> Result<()>
where
    T: Serialize,
{
    match format {
        OutputFormat::Json | OutputFormat::Human => {
            print_value(format, &json!({ root: items }))?;
        }
        OutputFormat::Files => {
            let value = serde_json::to_value(items)?;
            if let Some(entries) = value.as_array() {
                for entry in entries {
                    match entry {
                        serde_json::Value::String(value) => println!("{value}"),
                        _ => println!("{}", serde_json::to_string(entry)?),
                    }
                }
            }
        }
        OutputFormat::Csv => print_csv(items)?,
        OutputFormat::Markdown => print_markdown(items)?,
        OutputFormat::Xml => print_xml(root, items)?,
    }
    Ok(())
}

fn print_value(format: OutputFormat, value: &serde_json::Value) -> Result<()> {
    match format {
        OutputFormat::Json => print_json(value),
        OutputFormat::Human => println!("{}", serde_json::to_string_pretty(value)?),
        OutputFormat::Csv => print_csv_value(value)?,
        OutputFormat::Markdown => print_markdown_value(value)?,
        OutputFormat::Xml => print_xml_value("result", value)?,
        OutputFormat::Files => {
            if let Some(path) = value.get("file").and_then(|item| item.as_str()) {
                println!("{path}");
            } else {
                print_json(value);
            }
        }
    }
    Ok(())
}

fn print_json(value: &serde_json::Value) {
    println!("{value}");
}

fn print_csv<T>(items: &[T]) -> Result<()>
where
    T: Serialize,
{
    let values = items
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Ok(());
    }
    let headers = object_keys(&values[0])?;
    println!("{}", headers.join(","));
    for value in values {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow!("csv output requires object rows"))?;
        let row = headers
            .iter()
            .map(|header| csv_cell(object.get(header).unwrap_or(&serde_json::Value::Null)))
            .collect::<Vec<_>>();
        println!("{}", row.join(","));
    }
    Ok(())
}

fn print_csv_value(value: &serde_json::Value) -> Result<()> {
    if let Some(array) = value.as_array() {
        return print_csv(array);
    }
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("csv output requires object values"))?;
    let headers = object.keys().cloned().collect::<Vec<_>>();
    println!("{}", headers.join(","));
    let row = headers
        .iter()
        .map(|header| csv_cell(object.get(header).unwrap_or(&serde_json::Value::Null)))
        .collect::<Vec<_>>();
    println!("{}", row.join(","));
    Ok(())
}

fn print_markdown<T>(items: &[T]) -> Result<()>
where
    T: Serialize,
{
    for value in items.iter().map(serde_json::to_value) {
        let value = value?;
        println!("```json");
        println!("{}", serde_json::to_string_pretty(&value)?);
        println!("```");
    }
    Ok(())
}

fn print_markdown_value(value: &serde_json::Value) -> Result<()> {
    println!("```json");
    println!("{}", serde_json::to_string_pretty(value)?);
    println!("```");
    Ok(())
}

fn print_xml<T>(root: &str, items: &[T]) -> Result<()>
where
    T: Serialize,
{
    println!("<{root}>");
    for item in items.iter().map(serde_json::to_value) {
        print_xml_entry("item", &item?)?;
    }
    println!("</{root}>");
    Ok(())
}

fn print_xml_value(root: &str, value: &serde_json::Value) -> Result<()> {
    print_xml_entry(root, value)
}

fn print_xml_entry(tag: &str, value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Object(object) => {
            println!("<{tag}>");
            for (key, value) in object {
                print_xml_entry(key, value)?;
            }
            println!("</{tag}>");
        }
        serde_json::Value::Array(values) => {
            println!("<{tag}>");
            for value in values {
                print_xml_entry("item", value)?;
            }
            println!("</{tag}>");
        }
        serde_json::Value::Null => println!("<{tag} />"),
        _ => println!("<{tag}>{}</{tag}>", xml_escape(&scalar_to_string(value))),
    }
    Ok(())
}

fn object_keys(value: &serde_json::Value) -> Result<Vec<String>> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("formatter requires object values"))?;
    Ok(object.keys().cloned().collect::<Vec<_>>())
}

fn csv_cell(value: &serde_json::Value) -> String {
    let scalar = scalar_to_string(value);
    let escaped = scalar.replace('\"', "\"\"");
    format!("\"{escaped}\"")
}

fn scalar_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn file_value(file: &IndexedFile) -> serde_json::Value {
    json!({
        "docid": file.docid,
        "file": file.path,
        "virtual_path": file.virtual_path,
        "collection": file.collection,
        "kind": file.kind,
        "title": file.title,
        "language": file.language,
        "context": file.context,
        "content": file.content,
        "line_start": file.line_start,
        "line_end": file.line_end,
    })
}
