//! The CI workflow. Nothing else notices when one of its guards goes missing.

use std::path::PathBuf;

use yaml_rust2::{Yaml, YamlLoader};

fn workflow() -> Yaml {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/ci.yml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let mut documents =
        YamlLoader::load_from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    assert_eq!(documents.len(), 1, "ci.yml must hold one YAML document");
    documents.remove(0)
}

fn rust_steps() -> Vec<Yaml> {
    workflow()["jobs"]["rust"]["steps"]
        .as_vec()
        .expect("the rust job must have steps")
        .clone()
}

#[test]
fn the_workflow_is_valid_yaml_with_jobs() {
    assert!(workflow()["jobs"].as_hash().is_some());
}

#[test]
fn it_runs_on_pushes_to_master_and_on_pull_requests() {
    let workflow = workflow();
    let triggers = &workflow["on"];
    let branches = triggers["push"]["branches"]
        .as_vec()
        .expect("ci.yml must run on pushes to named branches");
    assert!(branches.contains(&Yaml::String("master".into())));
    assert!(
        !triggers["pull_request"].is_badvalue(),
        "ci.yml must run on pull requests"
    );
}

/// Cancelling by ref would apply to master as well: of two quick pushes the
/// first would be left without a CI result. Only pull request runs may
/// supersede each other.
#[test]
fn only_pull_request_runs_cancel_each_other() {
    let workflow = workflow();
    let cancel = &workflow["concurrency"]["cancel-in-progress"];
    assert!(
        cancel
            .as_str()
            .is_some_and(|condition| condition.contains("pull_request")),
        "cancel-in-progress must depend on the run being a pull request, not {cancel:?}"
    );
}

/// A hung run without a timeout burns GitHub's default of 360 minutes.
#[test]
fn every_job_has_a_timeout() {
    let workflow = workflow();
    let jobs = workflow["jobs"].as_hash().expect("ci.yml must have jobs");
    assert!(!jobs.is_empty());
    for (name, job) in jobs {
        assert!(
            job["timeout-minutes"].as_i64().is_some(),
            "job {name:?} must set timeout-minutes"
        );
    }
}

#[test]
fn the_rust_job_checks_formatting_lints_and_the_coverage_floor() {
    let steps = rust_steps();
    let commands: Vec<&str> = steps
        .iter()
        .filter_map(|step| step["run"].as_str())
        .collect();
    assert!(commands.contains(&"cargo fmt --check"), "{commands:?}");
    assert!(
        commands.contains(&"cargo clippy --all-targets -- -D warnings"),
        "{commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|run| run.starts_with("cargo llvm-cov") && run.contains("--fail-under-lines 80")),
        "the rust job must run the tests under the 80% line-coverage floor: {commands:?}"
    );
}

/// A fixed cache key without restore-keys is an exact hit forever once it is
/// saved: a partial entry would stick until someone deletes the cache.
#[test]
fn the_model_cache_can_recover_from_a_bad_entry() {
    let steps = rust_steps();
    let cache = steps
        .iter()
        .find(|step| {
            step["with"]["path"]
                .as_str()
                .is_some_and(|path| path.contains(".model-cache"))
        })
        .expect("the rust job must cache the embedding model");
    assert!(
        !cache["with"]["restore-keys"].is_badvalue(),
        "the model cache needs restore-keys, so that a new key still restores an earlier entry"
    );
}
