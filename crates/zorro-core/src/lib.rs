//! # zorro-core
//!
//! The headless engine behind [Zorro](https://github.com/baboons/zorro), a
//! native macOS Git merge-conflict resolver. This crate has **no UI and no
//! third-party dependencies** — it is the pure, testable core that the GPUI app
//! layer drives:
//!
//! - [`conflict`] — parse a conflicted file into a [`MergeDocument`], attach a
//!   [`Resolution`] to each conflict, and render resolved text back out.
//! - [`diff`] — word/character-level token diffing for in-conflict highlighting.
//! - [`session`] — model a whole [`MergeSession`] across many files.
//! - [`git`] — discover conflicts and the active workflow via the `git` CLI.
//!
//! ```
//! use zorro_core::conflict::{MergeDocument, Resolution};
//!
//! let conflicted = "\
//! line one
//! <<<<<<< HEAD
//! ours
//! =======
//! theirs
//! >>>>>>> feature
//! line three
//! ";
//!
//! let mut doc = MergeDocument::parse(conflicted);
//! assert_eq!(doc.conflict_count(), 1);
//!
//! doc.resolve(0, Resolution::Incoming);
//! assert!(doc.is_fully_resolved());
//! assert_eq!(doc.render(), "line one\ntheirs\nline three\n");
//! ```

pub mod ai;
pub mod conflict;
pub mod diff;
pub mod git;
pub mod session;
pub mod syntax;
pub mod update;
pub mod validate;

pub use ai::{AiConflict, AiError, AiProvider, CliProvider, Confidence, Suggestion};
pub use conflict::{Conflict, ConflictLabels, LineEnding, MergeDocument, Resolution, Section, Side};
pub use diff::{diff, line_diff_flags, DiffSpan, DiffTag, Granularity};
pub use session::{ConflictFile, FileKind, FileStatus, MergeSession, Workflow};
pub use syntax::{Highlighter, Language, Token, TokenKind};
pub use validate::{check as validate, SyntaxIssue};
