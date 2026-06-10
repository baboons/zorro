//! The merge *session*: the set of conflicted files a single Git operation left
//! behind, plus each file's resolution state. This is the model the UI drives —
//! the file tree, the "1 / 4 files completed" header, and per-file navigation.

use std::path::{Path, PathBuf};

use crate::conflict::MergeDocument;
use crate::syntax::Language;
use crate::validate::{self, SyntaxIssue};

/// What kind of operation produced the conflicts, detected from Git's state.
/// Drives the window title and any workflow-specific wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workflow {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    StashPop,
    Apply,
    /// Conflicts present but the originating operation is unknown.
    Unknown,
}

impl Workflow {
    pub fn label(self) -> &'static str {
        match self {
            Workflow::Merge => "Merge",
            Workflow::Rebase => "Rebase",
            Workflow::CherryPick => "Cherry-pick",
            Workflow::Revert => "Revert",
            Workflow::StashPop => "Stash pop",
            Workflow::Apply => "Apply",
            Workflow::Unknown => "Merge",
        }
    }
}

/// How Zorro classifies a conflicted path, which decides whether it gets the
/// text merge view or the binary keep-current / keep-incoming view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Text,
    Image,
    Binary,
}

impl FileKind {
    /// Best-effort classification from the file extension. Real binary sniffing
    /// (NUL-byte detection) happens when contents are loaded; this is the cheap
    /// first pass used while building the tree.
    pub fn from_path(path: &Path) -> FileKind {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "heic") => {
                FileKind::Image
            }
            Some(
                "pdf" | "zip" | "gz" | "tar" | "bin" | "exe" | "dll" | "dylib" | "so" | "a" | "o"
                | "class" | "jar" | "wasm" | "sketch" | "fig" | "psd" | "ai" | "mp4" | "mov"
                | "mp3" | "wav" | "woff" | "woff2" | "ttf" | "otf",
            ) => FileKind::Binary,
            _ => FileKind::Text,
        }
    }
}

/// The user-facing resolution status of a single file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// At least one conflict still needs a decision.
    Unresolved,
    /// Every conflict is resolved and the result is structurally sound.
    Resolved,
    /// Every conflict is resolved but the merged result has a structural problem
    /// (e.g. unbalanced brackets) the user must fix before continuing.
    Invalid,
}

/// One conflicted file in the session.
#[derive(Clone, Debug)]
pub struct ConflictFile {
    /// Path relative to the repository root, e.g. `src/auth.ts`.
    pub path: PathBuf,
    pub kind: FileKind,
    /// Parsed text document, present for [`FileKind::Text`] files.
    pub document: Option<MergeDocument>,
}

impl ConflictFile {
    /// Build a text file entry from already-loaded contents.
    pub fn from_text(path: impl Into<PathBuf>, contents: &str) -> ConflictFile {
        let path = path.into();
        ConflictFile {
            kind: FileKind::Text,
            document: Some(MergeDocument::parse(contents)),
            path,
        }
    }

    /// Build a binary/image file entry (no text document).
    pub fn binary(path: impl Into<PathBuf>, kind: FileKind) -> ConflictFile {
        ConflictFile {
            path: path.into(),
            kind,
            document: None,
        }
    }

    pub fn status(&self) -> FileStatus {
        match &self.document {
            Some(doc) if !doc.is_fully_resolved() => FileStatus::Unresolved,
            Some(doc) => {
                // Fully resolved: it's only "done" if the result is structurally
                // sound — otherwise the user must fix it before continuing.
                if validate::check(&doc.render(), Language::from_path(&self.path)).is_empty() {
                    FileStatus::Resolved
                } else {
                    FileStatus::Invalid
                }
            }
            // Binary files are unresolved until the user explicitly picks a side.
            None => FileStatus::Unresolved,
        }
    }

    /// Structural problems in the fully-resolved result, if any. Empty while the
    /// file still has unresolved conflicts (markers are expected then).
    pub fn syntax_issues(&self) -> Vec<SyntaxIssue> {
        match &self.document {
            Some(doc) if doc.is_fully_resolved() => {
                validate::check(&doc.render(), Language::from_path(&self.path))
            }
            _ => Vec::new(),
        }
    }

    /// File name component for display in the tree.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

/// A whole merge session across one or more conflicted files.
#[derive(Clone, Debug)]
pub struct MergeSession {
    pub workflow: Workflow,
    pub files: Vec<ConflictFile>,
}

impl MergeSession {
    pub fn new(workflow: Workflow, files: Vec<ConflictFile>) -> MergeSession {
        MergeSession { workflow, files }
    }

    /// Number of files whose conflicts are all resolved.
    pub fn completed_files(&self) -> usize {
        self.files
            .iter()
            .filter(|f| f.status() == FileStatus::Resolved)
            .count()
    }

    pub fn total_files(&self) -> usize {
        self.files.len()
    }

    /// Total outstanding conflicts across all text files.
    pub fn total_unresolved_conflicts(&self) -> usize {
        self.files
            .iter()
            .filter_map(|f| f.document.as_ref())
            .map(|d| d.unresolved_count())
            .sum()
    }

    /// Whether the entire session is done.
    pub fn is_complete(&self) -> bool {
        self.files.iter().all(|f| f.status() == FileStatus::Resolved)
    }

    /// Index of the first file that still needs attention, for "jump to next
    /// unresolved file".
    pub fn first_unresolved(&self) -> Option<usize> {
        self.files
            .iter()
            .position(|f| f.status() == FileStatus::Unresolved)
    }

    /// A short header line, e.g. `Merge · 1 / 4 files · 3 conflicts left`.
    pub fn summary(&self) -> String {
        let conflicts = self.total_unresolved_conflicts();
        let conflicts_part = match conflicts {
            0 => "no conflicts left".to_string(),
            1 => "1 conflict left".to_string(),
            n => format!("{n} conflicts left"),
        };
        format!(
            "{} · {} / {} files · {}",
            self.workflow.label(),
            self.completed_files(),
            self.total_files(),
            conflicts_part,
        )
    }
}
