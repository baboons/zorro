//! Integration test that drives the real `git` CLI: build a throwaway repo,
//! create an actual merge conflict, then assert the discovery layer finds it.
//!
//! Skips itself gracefully if `git` is unavailable in the environment.

use std::path::{Path, PathBuf};
use std::process::Command;

use zorro_core::git::Repository;
use zorro_core::session::{FileStatus, Workflow};

/// Run a git command in `dir`, panicking with context on failure.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        status.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&status.stderr)
    );
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A unique temp directory under the system temp dir, removed on drop.
struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new(tag: &str) -> TempRepo {
        let mut path = std::env::temp_dir();
        // Use the process id + tag for a stable-but-unique name (no rand dep).
        path.push(format!("zorro-it-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempRepo { path }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Create a repo with a single file, then two divergent branches that conflict
/// on merge. Leaves the work tree mid-merge with the conflict unresolved.
fn make_conflicted_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "test@zorro.dev"]);
    git(dir, &["config", "user.name", "Zorro Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    // Pin the baseline style so tests don't inherit a global diff3 setting.
    git(dir, &["config", "merge.conflictstyle", "merge"]);

    std::fs::write(dir.join("greeting.txt"), "hello base\n").unwrap();
    git(dir, &["add", "greeting.txt"]);
    git(dir, &["commit", "-q", "-m", "base"]);

    // Branch that changes the line.
    git(dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("greeting.txt"), "hello from feature\n").unwrap();
    git(dir, &["commit", "-q", "-am", "feature change"]);

    // Diverge main with a conflicting change.
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("greeting.txt"), "hello from main\n").unwrap();
    git(dir, &["commit", "-q", "-am", "main change"]);

    // Merge should conflict; ignore the expected non-zero exit.
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["merge", "feature"])
        .output()
        .expect("spawn git merge");
}

#[test]
fn discovers_conflicted_files_in_real_repo() {
    if !git_available() {
        eprintln!("skipping: git not available");
        return;
    }

    let repo = TempRepo::new("discover");
    make_conflicted_repo(&repo.path);

    let r = Repository::discover(&repo.path).expect("discover repo");
    let conflicts = r.conflicted_paths().expect("list conflicts");

    assert_eq!(conflicts, vec![PathBuf::from("greeting.txt")]);
    assert_eq!(r.current_workflow(), Workflow::Merge);
}

#[test]
fn load_session_parses_the_conflict() {
    if !git_available() {
        eprintln!("skipping: git not available");
        return;
    }

    let repo = TempRepo::new("session");
    make_conflicted_repo(&repo.path);

    let r = Repository::discover(&repo.path).expect("discover repo");
    let session = r.load_session().expect("load session");

    assert_eq!(session.total_files(), 1);
    assert_eq!(session.workflow, Workflow::Merge);

    let file = &session.files[0];
    assert_eq!(file.status(), FileStatus::Unresolved);

    let doc = file.document.as_ref().expect("text document");
    assert_eq!(doc.conflict_count(), 1);

    let c = doc.conflict(0).unwrap();
    assert_eq!(c.current, vec!["hello from main"]);
    assert_eq!(c.incoming, vec!["hello from feature"]);
}

#[test]
fn switching_to_diff3_regenerates_conflicts_with_base() {
    if !git_available() {
        return;
    }
    let repo = TempRepo::new("diff3");
    make_conflicted_repo(&repo.path);

    let r = Repository::discover(&repo.path).expect("discover repo");

    // Baseline `merge` style: no common ancestor in the markers.
    let before = r.load_session().expect("load session");
    let had_base = before.files[0]
        .document
        .as_ref()
        .unwrap()
        .conflicts()
        .any(|c| c.has_base());
    assert!(!had_base, "merge style should not include a base section");

    // Switch + regenerate, then the conflict should carry the base.
    r.set_conflict_style("diff3").expect("set style");
    r.regenerate_conflicts().expect("regenerate");
    assert_eq!(r.conflict_style().as_deref(), Some("diff3"));

    let after = r.load_session().expect("reload session");
    let has_base = after.files[0]
        .document
        .as_ref()
        .unwrap()
        .conflicts()
        .any(|c| c.has_base());
    assert!(has_base, "diff3 style should include a base section");
}

#[test]
fn stage_and_complete_commits_the_merge() {
    if !git_available() {
        return;
    }
    let repo = TempRepo::new("commit");
    make_conflicted_repo(&repo.path);
    let r = Repository::discover(&repo.path).expect("discover");
    assert_eq!(r.current_workflow(), Workflow::Merge);

    // Resolve by writing clean content, then stage + complete.
    std::fs::write(repo.path.join("greeting.txt"), "resolved\n").unwrap();
    r.stage(&[PathBuf::from("greeting.txt")]).expect("stage");
    r.complete(Workflow::Merge).expect("complete");

    // The merge is finished: no unmerged files, MERGE_HEAD gone, content kept.
    assert!(r.conflicted_paths().unwrap().is_empty());
    assert!(!r.git_dir().join("MERGE_HEAD").exists());
    assert_eq!(
        std::fs::read_to_string(repo.path.join("greeting.txt")).unwrap(),
        "resolved\n"
    );
}

#[test]
fn abort_restores_pre_merge_state() {
    if !git_available() {
        return;
    }
    let repo = TempRepo::new("abort");
    make_conflicted_repo(&repo.path);
    let r = Repository::discover(&repo.path).expect("discover");

    r.abort(Workflow::Merge).expect("abort");
    assert!(r.conflicted_paths().unwrap().is_empty());
    assert!(!r.git_dir().join("MERGE_HEAD").exists());
}

#[test]
fn discover_outside_repo_errors() {
    if !git_available() {
        return;
    }
    // The system temp dir itself is not (usually) a git repo.
    let tmp = TempRepo::new("norepo");
    let result = Repository::discover(&tmp.path);
    assert!(result.is_err(), "expected discovery to fail outside a repo");
}
