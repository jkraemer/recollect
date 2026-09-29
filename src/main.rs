use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;

use recollect::config::Config;
use recollect::filter::Filter;
use recollect::memory::{MemoryType, ProjectRef, normalize_tags};
use recollect::output;
use recollect::service::{Recollect, StoreInput};
use recollect::time::{parse_since, parse_until};

/// Persistent, searchable memory for coding agents.
#[derive(Parser)]
#[command(name = "recollect", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Store a memory; the content comes from the argument or from stdin
    Store(StoreArgs),
    /// Search memories by words and meaning
    Search(SearchArgs),
    /// List memories, newest first
    List(ListArgs),
    /// Show one memory in full
    Show {
        id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Delete a memory
    Delete { id: i64 },
    /// The latest session and the recent notes and todos
    Context {
        /// Project name; "global" selects memories without a project
        #[arg(short, long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Projects and their memory counts
    Projects {
        #[arg(long)]
        json: bool,
    },
    /// Tag frequencies
    Tags(TagsArgs),
    /// Embed memories that have no vectors yet
    Reindex {
        /// Discard every stored vector and embed all memories again
        #[arg(long)]
        all: bool,
    },
    /// Storage location, counts and vector health
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct StoreArgs {
    /// The memory; read from stdin when omitted
    content: Option<String>,
    /// Project name; omitted or "global" stores a memory without a project
    #[arg(short, long)]
    project: Option<String>,
    #[arg(short = 't', long = "type", value_enum, default_value_t = MemoryType::Note)]
    memory_type: MemoryType,
    /// Tags, comma-separated
    #[arg(short = 'T', long, value_delimiter = ',')]
    tags: Vec<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct FilterArgs {
    /// Project name; "global" selects memories without a project
    #[arg(short, long)]
    project: Option<String>,
    /// Memory types, comma-separated
    #[arg(short = 't', long = "type", value_enum, value_delimiter = ',')]
    types: Vec<MemoryType>,
    /// Only memories carrying all of these tags, comma-separated
    #[arg(short = 'T', long, value_delimiter = ',')]
    tags: Vec<String>,
    /// Created on or after (YYYY-MM-DD or RFC 3339)
    #[arg(long)]
    since: Option<String>,
    /// Created on or before (YYYY-MM-DD covers the whole day)
    #[arg(long)]
    until: Option<String>,
}

impl FilterArgs {
    fn to_filter(&self) -> recollect::Result<Filter> {
        Ok(Filter {
            project: self.project.as_deref().map(ProjectRef::parse).transpose()?,
            types: self.types.clone(),
            tags: normalize_tags(&self.tags)?,
            since: self.since.as_deref().map(parse_since).transpose()?,
            until: self.until.as_deref().map(parse_until).transpose()?,
        })
    }
}

#[derive(Args)]
struct SearchArgs {
    /// Search text; several words may be given without quotes
    #[arg(required = true, num_args = 1..)]
    query: Vec<String>,
    #[command(flatten)]
    filter: FilterArgs,
    #[arg(short, long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    filter: FilterArgs,
    #[arg(short, long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TagsArgs {
    /// Project name; "global" selects memories without a project
    #[arg(short, long)]
    project: Option<String>,
    /// Memory types, comma-separated
    #[arg(short = 't', long = "type", value_enum, value_delimiter = ',')]
    types: Vec<MemoryType>,
    /// How many tags to show
    #[arg(short = 'n', long, default_value_t = 20)]
    top: usize,
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if reader_went_away(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Whether writing to stdout failed because its reader closed the pipe, as
/// `recollect list | head -1` does; that ends the command without an error.
fn reader_went_away(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|err| err.kind() == std::io::ErrorKind::BrokenPipe)
}

fn run(command: Command) -> anyhow::Result<()> {
    let mut app = Recollect::open(Config::load()?)?;
    match command {
        Command::Store(args) => {
            let content = match args.content {
                Some(content) => content,
                None => read_stdin()?,
            };
            let project = match &args.project {
                Some(name) => ProjectRef::parse(name)?,
                None => ProjectRef::Global,
            };
            let outcome = app.store(StoreInput {
                content,
                project,
                memory_type: args.memory_type,
                tags: args.tags,
            })?;
            warn(outcome.warning.as_deref());
            if args.json {
                print_json(
                    &serde_json::json!({ "id": outcome.id, "global_id": outcome.global_id }),
                )?;
            } else {
                print_text(&format!("stored #{}", outcome.id))?;
            }
        }
        Command::Search(args) => {
            let outcome =
                app.search(&args.query.join(" "), &args.filter.to_filter()?, args.limit)?;
            warn(outcome.warning.as_deref());
            if args.json {
                print_json(&outcome.results)?;
            } else {
                print_text(&output::memory_blocks(
                    outcome.results.iter().map(|result| &result.memory),
                ))?;
            }
        }
        Command::List(args) => {
            let memories = app.list(&args.filter.to_filter()?, args.limit)?;
            if args.json {
                print_json(&memories)?;
            } else {
                print_text(&output::memory_blocks(&memories))?;
            }
        }
        Command::Show { id, json } => {
            let memory = app.show(id)?;
            if json {
                print_json(&memory)?;
            } else {
                print_text(&output::memory_block(&memory))?;
            }
        }
        Command::Delete { id } => {
            app.delete(id)?;
            print_text(&format!("deleted #{id}"))?;
        }
        Command::Context { project, json } => {
            let project = project.as_deref().map(ProjectRef::parse).transpose()?;
            let context = app.context(project.as_ref())?;
            if json {
                print_json(&context)?;
            } else {
                print_text(&output::context_text(&context))?;
            }
        }
        Command::Projects { json } => {
            let projects = app.projects()?;
            if json {
                print_json(&projects)?;
            } else {
                let rows: Vec<(&str, usize)> = projects
                    .iter()
                    .map(|p| (p.name.as_str(), p.count))
                    .collect();
                print_text(&output::counts_text(&rows))?;
            }
        }
        Command::Tags(args) => {
            let filter = Filter {
                project: args.project.as_deref().map(ProjectRef::parse).transpose()?,
                types: args.types,
                ..Filter::default()
            };
            let tags = app.tags(&filter, args.top)?;
            if args.json {
                print_json(&tags)?;
            } else {
                let rows: Vec<(&str, usize)> =
                    tags.iter().map(|t| (t.tag.as_str(), t.count)).collect();
                print_text(&output::counts_text(&rows))?;
            }
        }
        Command::Reindex { all } => {
            let count = app.reindex(all)?;
            print_text(&format!("embedded {count} memories"))?;
        }
        Command::Status { json } => {
            let status = app.status()?;
            if json {
                print_json(&status)?;
            } else {
                print_text(&output::status_text(&status))?;
            }
        }
    }
    Ok(())
}

fn read_stdin() -> anyhow::Result<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        Cli::command()
            .error(
                ErrorKind::MissingRequiredArgument,
                "no content: pass it as an argument or pipe it on stdin",
            )
            .exit();
    }
    let mut content = String::new();
    stdin
        .read_to_string(&mut content)
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::InvalidData => anyhow::anyhow!("content is not valid UTF-8"),
            _ => err.into(),
        })?;
    Ok(content)
}

fn warn(warning: Option<&str>) {
    if let Some(warning) = warning {
        eprintln!("warning: {warning}");
    }
}

/// Prints `text` and a newline; prints nothing at all for empty text.
fn print_text(text: &str) -> std::io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    writeln!(std::io::stdout().lock(), "{text}")
}

fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    Ok(print_text(&serde_json::to_string(value)?)?)
}
