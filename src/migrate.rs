//! Reads the memories of a Ruby recollect installation for `recollect
//! migrate-from-ruby`: `global.db` holds the global memories and each
//! `projects/<name>.db` those of project `<name>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::{Connection, OpenFlags};

use crate::error::{Error, Result};
use crate::memory::{
    MemoryType, NewRecord, ProjectRef, Tombstone, normalize_content, normalize_tags,
};
use crate::time::parse_timestamp;

/// The columns a Ruby `memories` table with rows must have.
const REQUIRED_COLUMNS: [&str; 5] = ["content", "memory_type", "tags", "created_at", "global_id"];

/// The Ruby chunker's pieces of long memories; their parents hold the whole text.
const CHUNK_TYPE: &str = "_chunk";

/// `--rename FROM=TO`: the memories of Ruby project `from` go to project `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

impl FromStr for Rename {
    type Err = String;

    fn from_str(raw: &str) -> std::result::Result<Self, String> {
        let (from, to) = raw
            .split_once('=')
            .ok_or_else(|| format!("expected FROM=TO, got {raw:?}"))?;
        Ok(Rename {
            from: from.to_string(),
            to: to.to_string(),
        })
    }
}

/// The memories of a Ruby installation: the live ones, ready to insert, and
/// the deleted ones.
#[derive(Debug, Default, PartialEq)]
pub struct RubyMemories {
    /// Oldest first, so the new ids follow the order the memories were written in.
    pub records: Vec<NewRecord>,
    pub chunks_skipped: usize,
    /// Memories deleted in Ruby; an import deletes them here too if an
    /// earlier run imported them.
    pub tombstones: Vec<Tombstone>,
}

/// Reads and checks every memory under the Ruby data directory `dir`, moving
/// the memories of each `--rename` source to its target project. A
/// `global_id` may appear only once across all files, live or deleted. Any
/// unexpected value fails the whole read, so nothing is dropped or mangled
/// silently. Files are opened read-only: a running Ruby server is not disturbed.
pub fn read_ruby_data(dir: &Path, renames: &[Rename]) -> Result<RubyMemories> {
    let files = ruby_files(dir)?;
    if files.is_empty() {
        return Err(Error::RubyData {
            path: dir.to_path_buf(),
            message: "no Ruby data: neither global.db nor projects/*.db".to_string(),
        });
    }
    check_renames(dir, &files, renames)?;
    let mut memories = RubyMemories::default();
    // Where each global id was read, so a second occurrence can name the first.
    let mut origins: HashMap<String, (PathBuf, i64)> = HashMap::new();
    for file in &files {
        let contents = file.read(renames).map_err(|err| file.with_path(err))?;
        memories.chunks_skipped += contents.chunks_skipped;
        let rows = contents
            .records
            .iter()
            .map(|(ruby_id, record)| (*ruby_id, &record.global_id))
            .chain(
                contents
                    .tombstones
                    .iter()
                    .map(|(ruby_id, tombstone)| (*ruby_id, &tombstone.global_id)),
            );
        for (ruby_id, global_id) in rows {
            if let Some((first_path, first_id)) =
                origins.insert(global_id.clone(), (file.path.clone(), ruby_id))
            {
                return Err(file.invalid(format!(
                    "memory {ruby_id}: global_id {global_id:?} is also memory {first_id} of {}",
                    first_path.display()
                )));
            }
        }
        memories
            .records
            .extend(contents.records.into_iter().map(|(_, record)| record));
        memories.tombstones.extend(
            contents
                .tombstones
                .into_iter()
                .map(|(_, tombstone)| tombstone),
        );
    }
    memories
        .records
        .sort_by(|a, b| (&a.created_at, &a.global_id).cmp(&(&b.created_at, &b.global_id)));
    Ok(memories)
}

/// A Ruby database file and the project name its file name gives.
struct RubyFile {
    path: PathBuf,
    /// The file name without `.db`; `None` for `global.db`.
    project_file: Option<String>,
}

/// What one Ruby file holds: its live and deleted memories, each with its
/// Ruby row id.
#[derive(Default)]
struct FileContents {
    records: Vec<(i64, NewRecord)>,
    chunks_skipped: usize,
    tombstones: Vec<(i64, Tombstone)>,
}

/// A Ruby row that is neither a chunk nor a tombstone, as stored.
struct LiveRow {
    memory_type: String,
    content: String,
    tags: Option<String>,
    created_at: Option<String>,
    global_id: Option<String>,
}

/// `global.db`, then `projects/*.db` by name; `-wal` and `-shm` files are no candidates.
fn ruby_files(dir: &Path) -> Result<Vec<RubyFile>> {
    let mut files = Vec::new();
    let global = dir.join("global.db");
    if global.is_file() {
        files.push(RubyFile {
            path: global,
            project_file: None,
        });
    }
    let projects = dir.join("projects");
    let entries = match std::fs::read_dir(&projects) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(err) => return Err(Error::file(&projects)(err)),
    };
    let mut project_files = Vec::new();
    for entry in entries {
        let path = entry.map_err(Error::file(&projects))?.path();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Not `Path::extension`: it finds none in `.db`, a file the Ruby server did create.
        if let Some(name) = file_name.strip_suffix(".db")
            && path.is_file()
        {
            project_files.push(RubyFile {
                project_file: Some(name.to_string()),
                path,
            });
        }
    }
    project_files.sort_by(|a, b| a.project_file.cmp(&b.project_file));
    files.extend(project_files);
    Ok(files)
}

/// Each `--rename` must name an existing project file, only once, and a valid target.
fn check_renames(dir: &Path, files: &[RubyFile], renames: &[Rename]) -> Result<()> {
    let invalid = |message: String| Error::RubyData {
        path: dir.join("projects"),
        message,
    };
    for (index, rename) in renames.iter().enumerate() {
        if !files
            .iter()
            .any(|file| file.project_file.as_deref() == Some(rename.from.as_str()))
        {
            return Err(invalid(format!("no project {:?} to rename", rename.from)));
        }
        if renames[..index]
            .iter()
            .any(|earlier| earlier.from == rename.from)
        {
            return Err(invalid(format!(
                "project {:?} is renamed twice",
                rename.from
            )));
        }
        ProjectRef::parse(&rename.to)?;
    }
    Ok(())
}

impl RubyFile {
    fn invalid(&self, message: impl Into<String>) -> Error {
        Error::RubyData {
            path: self.path.clone(),
            message: message.into(),
        }
    }

    /// Names this file in an SQLite error from reading it.
    fn with_path(&self, err: Error) -> Error {
        match err {
            Error::Database(err) => self.invalid(err.to_string()),
            other => other,
        }
    }

    /// The `project` column value of this file's memories: the file name,
    /// after `--rename`.
    fn project(&self, renames: &[Rename]) -> Result<Option<String>> {
        let Some(file_name) = &self.project_file else {
            return Ok(None);
        };
        let name = renames
            .iter()
            .find(|rename| rename.from == *file_name)
            .map_or(file_name.as_str(), |rename| rename.to.as_str());
        let project = ProjectRef::parse(name).map_err(|err| self.invalid(err.to_string()))?;
        Ok(project.column_value().map(String::from))
    }

    /// This file's memories. A file without rows, or without a `memories`
    /// table, is skipped before anything else about it is checked.
    fn read(&self, renames: &[Rename]) -> Result<FileContents> {
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let columns = memory_columns(&conn)?;
        let mut contents = FileContents::default();
        if columns.is_empty()
            || conn.query_row("SELECT count(*) FROM memories", [], |row| {
                row.get::<_, i64>(0)
            })? == 0
        {
            return Ok(contents);
        }
        let has = |name: &str| columns.iter().any(|column| column == name);
        if let Some(missing) = REQUIRED_COLUMNS.into_iter().find(|&name| !has(name)) {
            return Err(self.invalid(format!("the memories table has no {missing} column")));
        }
        let project = self.project(renames)?;
        let deleted_at_column = if has("deleted_at") {
            "deleted_at"
        } else {
            "NULL"
        };
        let mut statement = conn.prepare(&format!(
            "SELECT id, memory_type, {deleted_at_column}, content, tags, created_at, global_id
             FROM memories ORDER BY id"
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let ruby_id: i64 = row.get(0)?;
            let memory_type: String = row.get(1)?;
            if memory_type == CHUNK_TYPE {
                contents.chunks_skipped += 1;
            } else if let Some(deleted_at) = row.get::<_, Option<String>>(2)? {
                let tombstone = read_tombstone(row.get(6)?, &deleted_at)
                    .map_err(|message| self.invalid(format!("memory {ruby_id}: {message}")))?;
                contents.tombstones.push((ruby_id, tombstone));
            } else {
                let live = LiveRow {
                    memory_type,
                    content: row.get(3)?,
                    tags: row.get(4)?,
                    created_at: row.get(5)?,
                    global_id: row.get(6)?,
                };
                let record = live
                    .into_record(project.clone())
                    .map_err(|message| self.invalid(format!("memory {ruby_id}: {message}")))?;
                contents.records.push((ruby_id, record));
            }
        }
        Ok(contents)
    }
}

/// The column names of the file's `memories` table; empty when there is no such table.
fn memory_columns(conn: &Connection) -> Result<Vec<String>> {
    let mut statement = conn.prepare("SELECT name FROM pragma_table_info('memories')")?;
    let names = statement.query_map([], |row| row.get(0))?;
    Ok(names.collect::<rusqlite::Result<_>>()?)
}

/// The tombstone of a Ruby row deleted at `deleted_at`; the error says what
/// is wrong with the row.
fn read_tombstone(
    global_id: Option<String>,
    deleted_at: &str,
) -> std::result::Result<Tombstone, String> {
    Ok(Tombstone {
        global_id: global_id.ok_or("no global_id")?,
        deleted_at: parse_timestamp(deleted_at)
            .map_err(|_| format!("deleted_at {deleted_at:?} is not an RFC 3339 timestamp"))?,
        deleted_by_peer: None,
    })
}

impl LiveRow {
    /// The record to insert; the error says what is wrong with the row.
    fn into_record(self, project: Option<String>) -> std::result::Result<NewRecord, String> {
        let memory_type = MemoryType::from_db(&self.memory_type)
            .ok_or_else(|| format!("unknown memory type {:?}", self.memory_type))?;
        let global_id = self.global_id.ok_or("no global_id")?;
        let content = normalize_content(&self.content).map_err(|err| err.to_string())?;
        let tags = match self.tags {
            None => Vec::new(),
            Some(json) => {
                let raw: Vec<String> = serde_json::from_str(&json)
                    .map_err(|_| format!("tags {json:?} are not a JSON array of strings"))?;
                normalize_tags(&raw).map_err(|err| err.to_string())?
            }
        };
        let created_at = match self.created_at {
            None => return Err("no created_at".to_string()),
            Some(raw) => parse_timestamp(&raw)
                .map_err(|_| format!("created_at {raw:?} is not an RFC 3339 timestamp"))?,
        };
        Ok(NewRecord {
            global_id,
            project,
            memory_type,
            content,
            tags,
            origin_peer: None,
            created_at,
        })
    }
}
