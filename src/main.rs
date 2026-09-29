use clap::Parser;

/// Persistent, searchable memory for coding agents.
#[derive(Parser)]
#[command(name = "recollect", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
