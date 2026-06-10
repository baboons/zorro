//! Thin integration with the `git` CLI: locate the repository, discover the
//! files left conflicted by a failed merge, and detect which workflow produced
//! them. Everything here shells out to `git` rather than linking libgit2 — it
//! keeps the dependency footprint at zero and matches whatever Git the user runs.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::session::{ConflictFile, FileKind, MergeSession, Workflow};

/// Errors that can arise while talking to Git or reading the work tree.
#[derive(Debug)]
pub enum GitError {
    /// The `git` binary could not be launched.
    Spawn(io::Error),
    /// A git command exited non-zero; carries stderr for context.
    Command { args: String, stderr: String },
    /// The given path is not inside a Git work tree.
    NotARepository,
    /// Reading a work-tree file failed.
    Io(io::Error),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Spawn(e) => write!(f, "could not run git: {e}"),
            GitError::Command { args, stderr } => {
                write!(f, "`git {args}` failed: {}", stderr.trim())
            }
            GitError::NotARepository => write!(f, "not inside a git repository"),
            GitError::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for GitError {}

/// A handle to a Git repository rooted at its top-level work-tree directory.
#[derive(Clone, Debug)]
pub struct Repository {
    root: PathBuf,
    git_dir: PathBuf,
}

impl Repository {
    /// Discover the repository containing `start` (a file or directory inside it).
    pub fn discover(start: impl AsRef<Path>) -> Result<Repository, GitError> {
        let start = start.as_ref();
        let root = run_git(start, ["rev-parse", "--show-toplevel"])?;
        let root = root.trim();
        if root.is_empty() {
            return Err(GitError::NotARepository);
        }
        let git_dir = run_git(start, ["rev-parse", "--absolute-git-dir"])?;
        Ok(Repository {
            root: PathBuf::from(root),
            git_dir: PathBuf::from(git_dir.trim()),
        })
    }

    /// The work-tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.git` directory (may be outside `root` for worktrees/submodules).
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Paths (relative to the root) of files in a conflicted/unmerged state.
    ///
    /// Uses `git diff --name-only --diff-filter=U`, which lists exactly the
    /// files with unresolved conflicts regardless of the originating operation.
    pub fn conflicted_paths(&self) -> Result<Vec<PathBuf>, GitError> {
        let out = run_git(
            &self.root,
            ["diff", "--name-only", "--diff-filter=U", "-z"],
        )?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect())
    }

    /// Detect which workflow is currently in progress by probing for the marker
    /// files Git writes into the git directory.
    pub fn current_workflow(&self) -> Workflow {
        let g = &self.git_dir;
        if g.join("MERGE_HEAD").exists() {
            Workflow::Merge
        } else if g.join("rebase-merge").exists() || g.join("rebase-apply").exists() {
            Workflow::Rebase
        } else if g.join("CHERRY_PICK_HEAD").exists() {
            Workflow::CherryPick
        } else if g.join("REVERT_HEAD").exists() {
            Workflow::Revert
        } else {
            Workflow::Unknown
        }
    }

    /// The configured `merge.conflictStyle` (`merge`, `diff3`, or `zdiff3`), or
    /// `None` if unset (Git then defaults to `merge`).
    pub fn conflict_style(&self) -> Option<String> {
        run_git(&self.root, ["config", "--get", "merge.conflictstyle"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Set `merge.conflictStyle` for this repository.
    pub fn set_conflict_style(&self, style: &str) -> Result<(), GitError> {
        run_git(&self.root, ["config", "merge.conflictstyle", style]).map(|_| ())
    }

    /// Regenerate conflict markers for every unmerged file using the currently
    /// configured conflict style (e.g. after switching to diff3). This recreates
    /// the *same* conflicts — it does not change which lines conflict — so it is
    /// safe to run before any resolution has been written.
    pub fn regenerate_conflicts(&self) -> Result<(), GitError> {
        for rel in self.conflicted_paths()? {
            let path = rel.to_string_lossy().into_owned();
            // `checkout -m` re-materializes the conflict with the active style.
            let _ = run_git(&self.root, ["checkout", "-m", "--", path.as_str()]);
        }
        Ok(())
    }

    /// Stage the given (now-resolved) paths so Git treats them as merged.
    pub fn stage(&self, paths: &[PathBuf]) -> Result<(), GitError> {
        for path in paths {
            let p = path.to_string_lossy().into_owned();
            run_git(&self.root, ["add", "--", p.as_str()])?;
        }
        Ok(())
    }

    /// Finish the in-progress operation: commit the merge, or continue the
    /// rebase / cherry-pick / revert.
    pub fn complete(&self, workflow: Workflow) -> Result<(), GitError> {
        match workflow {
            Workflow::Rebase => self.run_noedit(&["rebase", "--continue"]),
            Workflow::CherryPick => self.run_noedit(&["cherry-pick", "--continue"]),
            Workflow::Revert => self.run_noedit(&["revert", "--continue"]),
            // Merge / stash / apply / unknown: a no-edit commit finalizes it.
            _ => run_git(&self.root, ["commit", "--no-edit"]).map(|_| ()),
        }
    }

    /// Abort the in-progress operation, restoring the pre-operation state.
    pub fn abort(&self, workflow: Workflow) -> Result<(), GitError> {
        let op = match workflow {
            Workflow::Rebase => "rebase",
            Workflow::CherryPick => "cherry-pick",
            Workflow::Revert => "revert",
            _ => "merge",
        };
        run_git(&self.root, [op, "--abort"]).map(|_| ())
    }

    /// Hard-reset the working tree to `HEAD`, discarding all uncommitted changes.
    pub fn reset_hard(&self) -> Result<(), GitError> {
        run_git(&self.root, ["reset", "--hard", "HEAD"]).map(|_| ())
    }

    /// Run a git subcommand with the editor suppressed (for `--continue` steps
    /// that would otherwise open `$EDITOR` for a commit message).
    fn run_noedit(&self, args: &[&str]) -> Result<(), GitError> {
        let output = Command::new("git")
            .current_dir(&self.root)
            .env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true")
            .args(args)
            .output()
            .map_err(GitError::Spawn)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(GitError::Command {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }

    /// Build a full [`MergeSession`] by discovering the conflicted files, loading
    /// the text ones, and classifying the rest.
    pub fn load_session(&self) -> Result<MergeSession, GitError> {
        let workflow = self.current_workflow();
        let mut files = Vec::new();
        for rel in self.conflicted_paths()? {
            let abs = self.root.join(&rel);
            let kind = FileKind::from_path(&rel);
            let file = match kind {
                FileKind::Text => match std::fs::read(&abs) {
                    Ok(bytes) if !is_binary(&bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        ConflictFile::from_text(rel, &text)
                    }
                    // Extension said text but bytes say binary (or unreadable):
                    // fall back to a binary entry so we never mis-parse.
                    Ok(_) => ConflictFile::binary(rel, FileKind::Binary),
                    Err(_) => ConflictFile::binary(rel, FileKind::Binary),
                },
                other => ConflictFile::binary(rel, other),
            };
            files.push(file);
        }
        Ok(MergeSession::new(workflow, files))
    }
}

/// Heuristic binary sniff: a NUL byte in the first 8 KiB means "binary".
fn is_binary(bytes: &[u8]) -> bool {
    let window = &bytes[..bytes.len().min(8192)];
    window.contains(&0)
}

/// Run a git subcommand in `cwd`, returning stdout on success.
fn run_git<I, S>(cwd: &Path, args: I) -> Result<String, GitError>
where
    I: IntoIterator<Item = S> + Clone,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args.clone())
        .output()
        .map_err(GitError::Spawn)?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // `rev-parse` outside a repo is the common, expected failure.
        if stderr.contains("not a git repository") {
            return Err(GitError::NotARepository);
        }
        let args = args
            .into_iter()
            .map(|s| s.as_ref().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        Err(GitError::Command { args, stderr })
    }
}
