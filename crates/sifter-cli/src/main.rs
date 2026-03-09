use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "sifter")]
#[command(author, version, about = "Local-first search for code and docs")]
struct Cli {}

fn main() {
    let _ = Cli::parse();
}
