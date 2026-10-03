//! The Claude Code plugin's files: manifests, hooks and skills.

use std::collections::BTreeMap;
use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::{Value, json};

const PLUGIN_VERSION: &str = "0.2.1";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn read_json(relative: &str) -> Value {
    serde_json::from_str(&read(relative)).unwrap_or_else(|err| panic!("{relative}: {err}"))
}

/// The `key: value` lines of a markdown file's frontmatter, between its leading `---` lines.
fn frontmatter(relative: &str) -> BTreeMap<String, String> {
    let text = read(relative);
    let (block, _) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .unwrap_or_else(|| panic!("{relative} has no frontmatter"));
    block
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

#[test]
fn the_manifests_name_the_plugin_and_agree_on_its_version() {
    let plugin = read_json(".claude-plugin/plugin.json");
    assert_eq!(plugin["name"], "recollect");
    assert_eq!(plugin["version"], PLUGIN_VERSION);
    let description = plugin["description"].as_str().unwrap();
    assert!(!description.contains("MCP"), "{description}");
    let marketplace = read_json(".claude-plugin/marketplace.json");
    assert_eq!(marketplace["metadata"]["version"], PLUGIN_VERSION);
    assert!(!marketplace["owner"]["name"].as_str().unwrap().is_empty());
    let entry = marketplace["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["name"] == "recollect")
        .expect("the marketplace lists the recollect plugin");
    assert_eq!(entry["source"], "./");
    let description = entry["description"].as_str().unwrap();
    assert!(
        description.contains("recollect binary on PATH"),
        "{description}"
    );
}

#[test]
fn the_plugin_has_no_mcp_server() {
    assert!(!root().join(".mcp.json").exists());
}

#[test]
fn the_hooks_run_the_recollect_cli() {
    let hooks = read_json("hooks/hooks.json");
    assert_eq!(
        hooks,
        json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup|clear|compact",
                    "hooks": [{
                        "type": "command",
                        "command": "recollect",
                        "args": ["hook", "session-start"],
                        "timeout": 10,
                        "statusMessage": "Loading memories"
                    }]
                }],
                "PostCompact": [{
                    "hooks": [{
                        "type": "command",
                        "command": "recollect",
                        "args": ["hook", "post-compact"],
                        "timeout": 60
                    }]
                }]
            }
        })
    );
}

/// Runs every hook the way Claude Code does: the command with its args and
/// the hook input on stdin. Neither hook loads the model for this input.
#[test]
fn every_hook_command_runs() {
    let hooks = read_json("hooks/hooks.json");
    let data = tempfile::tempdir().unwrap();
    let mut ran = 0;
    for groups in hooks["hooks"].as_object().unwrap().values() {
        for group in groups.as_array().unwrap() {
            for hook in group["hooks"].as_array().unwrap() {
                let args: Vec<&str> = hook["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap())
                    .collect();
                Command::cargo_bin(hook["command"].as_str().unwrap())
                    .unwrap()
                    .env("RECOLLECT_DATA_DIR", data.path())
                    .env("RECOLLECT_MODEL_DIR", data.path().join("models"))
                    .args(&args)
                    .write_stdin(json!({"cwd": data.path()}).to_string())
                    .assert()
                    .success()
                    .stderr("");
                ran += 1;
            }
        }
    }
    assert_eq!(ran, 2);
}

#[test]
fn the_skills_have_their_frontmatter() {
    let memory = frontmatter("skills/using-long-term-memory/SKILL.md");
    assert_eq!(
        memory.get("name").map(String::as_str),
        Some("using-long-term-memory")
    );
    assert!(
        memory
            .get("description")
            .is_some_and(|text| !text.is_empty())
    );
    let session_log = frontmatter("skills/session-log/SKILL.md");
    assert_eq!(
        session_log.get("name").map(String::as_str),
        Some("session-log")
    );
    assert!(
        session_log
            .get("description")
            .is_some_and(|text| !text.is_empty())
    );
    assert_eq!(
        session_log
            .get("disable-model-invocation")
            .map(String::as_str),
        Some("true"),
        "/session-log runs only when the user types it"
    );
    assert_eq!(
        session_log.get("allowed-tools").map(String::as_str),
        Some("Bash(recollect store *)")
    );
}

/// Commands are the older plugin format; skills supersede them.
#[test]
fn the_plugin_ships_skills_and_no_commands() {
    assert!(!root().join("commands").exists());
}

#[test]
fn the_skills_work_through_the_cli() {
    for file in [
        "skills/using-long-term-memory/SKILL.md",
        "skills/session-log/SKILL.md",
    ] {
        let text = read(file);
        assert!(text.contains("recollect store -p"), "{file}");
        for mcp in ["MCP", "store_memory", "search_memory", "get_context"] {
            assert!(!text.contains(mcp), "{file} mentions {mcp}");
        }
    }
}
