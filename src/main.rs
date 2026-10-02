use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand};
use serde::Serialize;

use recollect::config::Config;
use recollect::detect::detect_project;
use recollect::filter::Filter;
use recollect::hook::{
    COMPACTION_TAG, HookInput, compaction_summary_body, session_header_text, session_start_text,
};
use recollect::memory::{MemoryType, ProjectRef, normalize_tags};
use recollect::migrate::{Rename, read_ruby_data};
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
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Delete a memory
    Delete { id: i64 },
    /// The latest session and the recent notes and todos
    Context {
        #[command(flatten)]
        project: ProjectArg,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Projects and their memory counts
    Projects {
        #[command(flatten)]
        output: OutputArgs,
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
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Copy the memories of a Ruby recollect installation into this database
    MigrateFromRuby {
        /// The Ruby server's data directory, holding global.db and projects/
        ruby_data_dir: PathBuf,
        /// Store the memories of Ruby project FROM under project TO; repeatable
        #[arg(long, value_name = "FROM=TO")]
        rename: Vec<Rename>,
    },
    /// Run a Claude Code hook on the hook's JSON input from stdin
    #[command(subcommand)]
    Hook(HookEvent),
}

/// The Claude Code hook events recollect handles.
#[derive(Subcommand)]
enum HookEvent {
    /// Print the memory of the session directory's project (after a compaction, only its header)
    SessionStart,
    /// Store Claude Code's compaction summary as a session memory of the project
    PostCompact,
}

#[derive(Args)]
struct OutputArgs {
    /// Print JSON instead of text
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct StoreArgs {
    /// The memory; read from stdin when omitted (put -- before content starting with -)
    content: Option<String>,
    /// Project name; omitted or "global" stores a memory without a project
    #[arg(short, long)]
    project: Option<String>,
    /// Memory type
    #[arg(short = 't', long = "type", value_enum, default_value_t = MemoryType::Note)]
    memory_type: MemoryType,
    /// Tags, comma-separated
    #[arg(short = 'T', long, value_delimiter = ',')]
    tags: Vec<String>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct ProjectArg {
    /// Project name; "global" selects memories without a project
    #[arg(short, long)]
    project: Option<String>,
}

impl ProjectArg {
    fn parse(&self) -> recollect::Result<Option<ProjectRef>> {
        self.project.as_deref().map(ProjectRef::parse).transpose()
    }
}

/// The project and memory types a command covers.
#[derive(Args)]
struct ScopeArgs {
    #[command(flatten)]
    project: ProjectArg,
    /// Memory types, comma-separated
    #[arg(short = 't', long = "type", value_enum, value_delimiter = ',')]
    types: Vec<MemoryType>,
}

impl ScopeArgs {
    fn to_filter(&self) -> recollect::Result<Filter> {
        Ok(Filter {
            project: self.project.parse()?,
            types: self.types.clone(),
            ..Filter::default()
        })
    }
}

#[derive(Args)]
struct FilterArgs {
    #[command(flatten)]
    scope: ScopeArgs,
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
        let scope = self.scope.to_filter()?;
        Ok(Filter {
            tags: normalize_tags(&self.tags)?,
            since: self.since.as_deref().map(parse_since).transpose()?,
            until: self.until.as_deref().map(parse_until).transpose()?,
            ..scope
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
    /// How many results to show
    #[arg(short, long, default_value_t = 10)]
    limit: usize,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    filter: FilterArgs,
    /// How many memories to show
    #[arg(short, long, default_value_t = 20)]
    limit: usize,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct TagsArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    /// How many tags to show
    #[arg(short = 'n', long, default_value_t = 20)]
    top: usize,
    #[command(flatten)]
    output: OutputArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if reader_went_away(&err) => ExitCode::SUCCESS,
        Err(err) => {
            output::print_diagnostic(&format!("error: {err}"));
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
    if let Command::MigrateFromRuby {
        ruby_data_dir,
        rename,
    } = &command
    {
        return migrate_from_ruby(ruby_data_dir, rename);
    }
    let mut app = open_app()?;
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
            if args.output.json {
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
            if args.output.json {
                print_json(&outcome.results)?;
            } else {
                print_text(&output::memory_blocks(
                    outcome.results.iter().map(|result| &result.memory),
                ))?;
            }
        }
        Command::List(args) => {
            let memories = app.list(&args.filter.to_filter()?, args.limit)?;
            if args.output.json {
                print_json(&memories)?;
            } else {
                print_text(&output::memory_blocks(&memories))?;
            }
        }
        Command::Show { id, output } => {
            let memory = app.show(id)?;
            if output.json {
                print_json(&memory)?;
            } else {
                print_text(&output::memory_block(&memory))?;
            }
        }
        Command::Delete { id } => {
            app.delete(id)?;
            print_text(&format!("deleted #{id}"))?;
        }
        Command::Context { project, output } => {
            let context = app.context(project.parse()?.as_ref())?;
            if output.json {
                print_json(&context)?;
            } else {
                print_text(&output::context_text(&context))?;
            }
        }
        Command::Projects { output } => {
            let projects = app.projects()?;
            if output.json {
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
            let tags = app.tags(&args.scope.to_filter()?, args.top)?;
            if args.output.json {
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
        Command::Status { output } => {
            let status = app.status()?;
            if output.json {
                print_json(&status)?;
            } else {
                print_text(&output::status_text(&status))?;
            }
        }
        Command::Hook(event) => run_hook(event, &mut app)?,
        Command::MigrateFromRuby { .. } => {
            unreachable!("migrate-from-ruby returned before the database opened")
        }
    }
    Ok(())
}

fn open_app() -> anyhow::Result<Recollect> {
    Ok(Recollect::open(Config::load()?)?.with_notices(output::print_diagnostic))
}

/// Reads and checks all Ruby data before the database opens, so bad source
/// data leaves the data directory untouched.
fn migrate_from_ruby(ruby_data_dir: &Path, renames: &[Rename]) -> anyhow::Result<()> {
    let memories = read_ruby_data(ruby_data_dir, renames)?;
    let outcome = open_app()?.import(&memories.records, &memories.tombstones)?;
    print_text(&format!(
        "imported {} memories ({} already present, {} chunk rows skipped)",
        outcome.imported, outcome.already_present, memories.chunks_skipped
    ))?;
    print_text(&format!(
        "deleted {} memories ({} Ruby tombstones)",
        outcome.deleted,
        memories.tombstones.len()
    ))?;
    warn(outcome.warning.as_deref());
    print_text(&format!("embedded {} memories", outcome.embedded))?;
    Ok(())
}

/// Runs the hook for `event` on the hook input piped to stdin. The session
/// directory is the input's `cwd`, else the working directory.
fn run_hook(event: HookEvent, app: &mut Recollect) -> anyhow::Result<()> {
    let input = HookInput::parse(&std::io::read_to_string(std::io::stdin())?)?;
    let after_compaction = input.after_compaction();
    let dir = match input.cwd {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    match event {
        HookEvent::SessionStart => {
            let detection = detect_project(&dir)?;
            let text = if after_compaction {
                session_header_text(&detection)
            } else {
                session_start_text(&detection, &app.context(detection.project())?)
            };
            print_text(&text)?;
        }
        HookEvent::PostCompact => {
            let Some(summary) = input
                .compact_summary
                .as_deref()
                .map(compaction_summary_body)
                .filter(|summary| !summary.is_empty())
            else {
                return Ok(());
            };
            let project = detect_project(&dir)?
                .project()
                .cloned()
                .unwrap_or(ProjectRef::Global);
            let outcome = app.store(StoreInput {
                content: summary.to_string(),
                project,
                memory_type: MemoryType::Session,
                tags: vec![COMPACTION_TAG.to_string()],
            })?;
            warn(outcome.warning.as_deref());
        }
    }
    Ok(())
}

fn read_stdin() -> anyhow::Result<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        no_content_error().exit();
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

/// The usage error for `store` without content and without piped input.
fn no_content_error() -> clap::Error {
    let mut cli = Cli::command();
    // Building gives subcommands their full name for the usage line.
    cli.build();
    cli.find_subcommand_mut("store")
        .expect("store is a subcommand")
        .error(
            ErrorKind::MissingRequiredArgument,
            "no content: pass it as an argument or pipe it on stdin",
        )
}

fn warn(warning: Option<&str>) {
    if let Some(warning) = warning {
        output::print_diagnostic(&format!("warning: {warning}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storing_without_content_shows_the_store_usage() {
        let message = no_content_error().to_string();
        assert!(
            message.contains("Usage: recollect store [OPTIONS] [CONTENT]"),
            "{message}"
        );
    }
}
