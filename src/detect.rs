//! Which project a session directory belongs to: the nearest
//! `.recollect-project` file, else the git repository's directory name.
//! Detection reads the filesystem only; it never runs git.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::memory::ProjectRef;

/// The file whose first non-empty line names the project of its directory
/// and everything below it, up to the repository root; a worktree also takes
/// the one at its main checkout's root.
pub const PROJECT_FILE: &str = ".recollect-project";

/// Where a detected project's name came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSource {
    /// The repository's directory; in a worktree, the main repository's.
    Repository(PathBuf),
    /// A `.recollect-project` file.
    ProjectFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    Found {
        project: ProjectRef,
        source: ProjectSource,
    },
    /// No project; `reason` says why, as a sentence.
    NotFound { reason: String },
}

impl Detection {
    pub fn project(&self) -> Option<&ProjectRef> {
        match self {
            Detection::Found { project, .. } => Some(project),
            Detection::NotFound { .. } => None,
        }
    }
}

/// Detects the project of the absolute directory `dir`. Walking up from it,
/// the first `.recollect-project` file wins; the walk ends at the repository
/// root (the first directory holding `.git`) or at the filesystem root. When
/// the walk finds no file, a worktree takes the main checkout's
/// `.recollect-project` if it has one, and otherwise the repository root's
/// directory name (the main checkout's, for a worktree) is the project. Fails
/// only when a `.recollect-project` file exists but cannot be read.
pub fn detect_project(dir: &Path) -> Result<Detection> {
    for candidate in dir.ancestors() {
        let project_file = candidate.join(PROJECT_FILE);
        if project_file.exists() {
            return from_project_file(project_file);
        }
        if candidate.join(".git").exists() {
            return from_repository(candidate);
        }
    }
    Ok(Detection::NotFound {
        reason: format!(
            "{} is not in a git repository and has no {PROJECT_FILE} file.",
            dir.display()
        ),
    })
}

fn from_project_file(file: PathBuf) -> Result<Detection> {
    let text = std::fs::read_to_string(&file).map_err(Error::file(&file))?;
    let name = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    Ok(match ProjectRef::parse(name) {
        Ok(project) => Detection::Found {
            project,
            source: ProjectSource::ProjectFile(file),
        },
        Err(_) => Detection::NotFound {
            reason: format!(
                "{} names {name:?}, which is not a valid project name (allowed are a-z 0-9 . _ -).",
                file.display()
            ),
        },
    })
}

/// The project of the repository rooted at `root` when no `.recollect-project`
/// file lies between it and the session directory. A worktree shares its
/// project with the main checkout, so the main checkout's file applies and
/// its directory name stands in for the worktree's own.
fn from_repository(root: &Path) -> Result<Detection> {
    let main = main_repository(root);
    if let Some(main) = &main {
        let file = main.join(PROJECT_FILE);
        if file.exists() {
            return from_project_file(file);
        }
    }
    let repository = main.unwrap_or_else(|| root.to_path_buf());
    let name = repository
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(match ProjectRef::parse(&name) {
        Ok(project) => Detection::Found {
            project,
            source: ProjectSource::Repository(repository),
        },
        Err(_) => Detection::NotFound {
            reason: format!(
                "The repository directory name {name:?} is not a valid project name (allowed are a-z 0-9 . _ -); put a project name in {}.",
                repository.join(PROJECT_FILE).display()
            ),
        },
    })
}

/// The main repository of the worktree at `root`: its `.git` file points
/// (`gitdir:`) at a directory holding a `commondir` file, and the main
/// repository is the parent of that common git directory. `None` for
/// anything else, such as a plain repository or a submodule.
fn main_repository(root: &Path) -> Option<PathBuf> {
    let dot_git = std::fs::read_to_string(root.join(".git")).ok()?;
    let gitdir = root.join(dot_git.lines().next()?.strip_prefix("gitdir:")?.trim());
    let commondir = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let common = std::fs::canonicalize(gitdir.join(commondir.trim())).ok()?;
    common.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A directory that detection sees as a repository root: it holds a `.git` directory.
    fn repository(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        fs::create_dir_all(root.join(".git")).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A worktree of `main` at `main/.worktrees/<name>`, laid out as `git
    /// worktree add` does; `relative` writes a relative `gitdir:` path.
    fn worktree(main: &Path, name: &str, relative: bool) -> PathBuf {
        let gitdir = main.join(".git/worktrees").join(name);
        write(&gitdir.join("commondir"), "../..\n");
        let dir = main.join(".worktrees").join(name);
        let pointer = if relative {
            format!("../../.git/worktrees/{name}")
        } else {
            gitdir.display().to_string()
        };
        write(&dir.join(".git"), &format!("gitdir: {pointer}\n"));
        dir
    }

    fn found_in_repository(name: &str, dir: &Path) -> Detection {
        Detection::Found {
            project: ProjectRef::Named(name.to_string()),
            source: ProjectSource::Repository(dir.to_path_buf()),
        }
    }

    fn found_in_file(name: &str, file: &Path) -> Detection {
        Detection::Found {
            project: ProjectRef::Named(name.to_string()),
            source: ProjectSource::ProjectFile(file.to_path_buf()),
        }
    }

    #[test]
    fn a_repository_names_the_project_from_its_root_and_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let root = repository(dir.path(), "Fera");
        let expected = found_in_repository("fera", &root);
        assert_eq!(detect_project(&root).unwrap(), expected);
        assert_eq!(detect_project(&root.join("src/billing")).unwrap(), expected);
        assert_eq!(
            expected.project(),
            Some(&ProjectRef::Named("fera".to_string()))
        );
    }

    #[test]
    fn a_worktree_and_its_subdirectories_belong_to_the_main_repository() {
        let dir = tempfile::tempdir().unwrap();
        let main = repository(dir.path(), "planio");
        let expected = found_in_repository("planio", &fs::canonicalize(&main).unwrap());
        for (name, relative) in [("absolute", false), ("relative", true)] {
            let tree = worktree(&main, name, relative);
            assert_eq!(detect_project(&tree).unwrap(), expected, "{name}");
            assert_eq!(
                detect_project(&tree.join("app/models")).unwrap(),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn a_worktree_uses_the_main_checkouts_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let main = repository(dir.path(), "main-checkout");
        write(&main.join(PROJECT_FILE), "planio6\n");
        let tree = worktree(&main, "feat", false);
        let expected = found_in_file(
            "planio6",
            &fs::canonicalize(&main).unwrap().join(PROJECT_FILE),
        );
        assert_eq!(detect_project(&tree).unwrap(), expected);
        assert_eq!(detect_project(&tree.join("sub")).unwrap(), expected);
    }

    #[test]
    fn a_worktrees_own_project_file_wins_over_the_main_checkouts() {
        let dir = tempfile::tempdir().unwrap();
        let main = repository(dir.path(), "main-checkout");
        write(&main.join(PROJECT_FILE), "planio6\n");
        let tree = worktree(&main, "feat", false);
        let own = tree.join(PROJECT_FILE);
        write(&own, "feature-proj\n");
        let expected = found_in_file("feature-proj", &own);
        assert_eq!(detect_project(&tree).unwrap(), expected);
        assert_eq!(detect_project(&tree.join("sub")).unwrap(), expected);
    }

    #[test]
    fn a_submodule_is_its_own_project() {
        let dir = tempfile::tempdir().unwrap();
        let main = repository(dir.path(), "app");
        fs::create_dir_all(main.join(".git/modules/vendor-lib")).unwrap();
        let submodule = main.join("vendor/vendor-lib");
        write(
            &submodule.join(".git"),
            "gitdir: ../../.git/modules/vendor-lib\n",
        );
        assert_eq!(
            detect_project(&submodule).unwrap(),
            found_in_repository("vendor-lib", &submodule)
        );
    }

    #[test]
    fn a_project_file_overrides_the_repository_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = repository(dir.path(), "checkout");
        let file = root.join(PROJECT_FILE);
        write(&file, "\n   My.Project  \nignored\n");
        assert_eq!(
            detect_project(&root.join("src")).unwrap(),
            found_in_file("my.project", &file)
        );
    }

    #[test]
    fn the_nearest_project_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = repository(dir.path(), "mono");
        let web = root.join("web").join(PROJECT_FILE);
        write(&web, "web-app\n");
        assert_eq!(
            detect_project(&root.join("web/src")).unwrap(),
            found_in_file("web-app", &web)
        );
        assert_eq!(
            detect_project(&root.join("api")).unwrap(),
            found_in_repository("mono", &root)
        );
    }

    #[test]
    fn a_project_file_above_the_repository_root_does_not_apply() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(PROJECT_FILE), "outer\n");
        let root = repository(dir.path(), "inner");
        assert_eq!(
            detect_project(&root).unwrap(),
            found_in_repository("inner", &root)
        );
    }

    #[test]
    fn a_project_file_outside_any_repository_names_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        let file = notes.join(PROJECT_FILE);
        write(&file, "global\n");
        assert_eq!(
            detect_project(&notes.join("2026")).unwrap(),
            Detection::Found {
                project: ProjectRef::Global,
                source: ProjectSource::ProjectFile(file),
            }
        );
    }

    #[test]
    fn without_a_repository_or_project_file_there_is_no_project() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = dir.path().join("scratch");
        let detection = detect_project(&scratch).unwrap();
        assert_eq!(
            detection,
            Detection::NotFound {
                reason: format!(
                    "{} is not in a git repository and has no .recollect-project file.",
                    scratch.display()
                ),
            }
        );
        assert_eq!(detection.project(), None);
    }

    #[test]
    fn invalid_names_give_no_project_and_say_why() {
        let dir = tempfile::tempdir().unwrap();
        let root = repository(dir.path(), "My Repo");
        assert_eq!(
            detect_project(&root).unwrap(),
            Detection::NotFound {
                reason: format!(
                    "The repository directory name \"My Repo\" is not a valid project name (allowed are a-z 0-9 . _ -); put a project name in {}.",
                    root.join(PROJECT_FILE).display()
                ),
            }
        );
        let sub = root.join("sub");
        let file = sub.join(PROJECT_FILE);
        for (text, name) in [("bad name!\n", "bad name!"), ("\n  \n", "")] {
            write(&file, text);
            assert_eq!(
                detect_project(&sub).unwrap(),
                Detection::NotFound {
                    reason: format!(
                        "{} names {name:?}, which is not a valid project name (allowed are a-z 0-9 . _ -).",
                        file.display()
                    ),
                },
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_invalid_main_checkout_name_points_at_the_main_checkouts_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let main = repository(dir.path(), "My Repo");
        let tree = worktree(&main, "feat", false);
        assert_eq!(
            detect_project(&tree).unwrap(),
            Detection::NotFound {
                reason: format!(
                    "The repository directory name \"My Repo\" is not a valid project name (allowed are a-z 0-9 . _ -); put a project name in {}.",
                    fs::canonicalize(&main)
                        .unwrap()
                        .join(PROJECT_FILE)
                        .display()
                ),
            }
        );
    }

    #[test]
    fn an_unreadable_project_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(PROJECT_FILE);
        fs::create_dir(&file).unwrap();
        let err = detect_project(dir.path()).unwrap_err();
        assert!(
            matches!(&err, Error::File { path, .. } if *path == file),
            "{err}"
        );
    }
}
