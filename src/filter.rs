//! Memory filters shared by list, search and tag statistics.

use rusqlite::types::Value;

use crate::memory::{MemoryType, ProjectRef};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    /// `None` means every project, including global.
    pub project: Option<ProjectRef>,
    /// Empty means every type.
    pub types: Vec<MemoryType>,
    /// Memories must carry all of these normalized tags.
    pub tags: Vec<String>,
    /// Inclusive lower bound on `created_at`, in stored timestamp format.
    pub since: Option<String>,
    /// Inclusive upper bound on `created_at`, in stored timestamp format.
    pub until: Option<String>,
}

impl Filter {
    /// Conditions on the `memories` table aliased `m` (live memories only),
    /// joined with AND, plus their `?` parameters in order.
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let mut conditions = vec!["m.deleted_at IS NULL".to_string()];
        let mut params = Vec::new();
        match &self.project {
            None => {}
            Some(ProjectRef::Global) => conditions.push("m.project IS NULL".to_string()),
            Some(ProjectRef::Named(name)) => {
                conditions.push("m.project = ?".to_string());
                params.push(Value::Text(name.clone()));
            }
        }
        if !self.types.is_empty() {
            let marks = vec!["?"; self.types.len()].join(", ");
            conditions.push(format!("m.memory_type IN ({marks})"));
            params.extend(
                self.types
                    .iter()
                    .map(|t| Value::Text(t.as_str().to_string())),
            );
        }
        for tag in &self.tags {
            conditions.push(
                "EXISTS (SELECT 1 FROM json_each(m.tags) AS ft WHERE ft.value = ?)".to_string(),
            );
            params.push(Value::Text(tag.clone()));
        }
        if let Some(since) = &self.since {
            conditions.push("m.created_at >= ?".to_string());
            params.push(Value::Text(since.clone()));
        }
        if let Some(until) = &self.until {
            conditions.push("m.created_at <= ?".to_string());
            params.push(Value::Text(until.clone()));
        }
        (conditions.join(" AND "), params)
    }
}
