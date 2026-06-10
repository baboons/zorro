//! Parsing, modelling, and re-rendering of Git merge conflicts.
//!
//! A file produced by a failed Git merge is a flat sequence of *sections*:
//! ordinary [`Section::Stable`] runs of lines, interleaved with
//! [`Section::Conflict`] regions delimited by conflict markers. This module
//! turns that text into a structured [`MergeDocument`], lets callers attach a
//! [`Resolution`] to each conflict, and renders the document back to text —
//! emitting the chosen side for resolved conflicts and the original markers for
//! the ones still outstanding.
//!
//! Both marker styles Git can emit are understood:
//!
//! ```text
//! <<<<<<< HEAD            <<<<<<< HEAD
//! current                current
//! =======                ||||||| merged common ancestors
//!                        base
//! incoming               =======
//! >>>>>>> branch          incoming
//!                        >>>>>>> branch
//!   (merge style)            (diff3 / zdiff3 style)
//! ```

use std::fmt;

/// The minimum run length Git uses for a conflict marker. Git defaults to seven
/// characters and only ever lengthens them for the (rare) nested case; we treat
/// "seven or more" as a marker so configured `conflict-marker-size` still parses.
const MARKER_LEN: usize = 7;

/// Which line terminator a file uses. Preserved across a parse/render round-trip
/// so resolving a CRLF file does not silently rewrite every line ending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }
}

/// The three sides of a conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// "Ours" — the branch being merged *into* (typically `HEAD`).
    Current,
    /// The common ancestor. Only present in diff3/zdiff3 style.
    Base,
    /// "Theirs" — the branch being merged *in*.
    Incoming,
}

/// The branch/ref labels Git wrote next to each marker, kept verbatim so an
/// unresolved conflict re-renders byte-for-byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictLabels {
    /// Label after `<<<<<<<`, e.g. `HEAD`.
    pub current: String,
    /// Label after `|||||||`, if the file was produced in diff3 style.
    pub base: Option<String>,
    /// Label after `>>>>>>>`, e.g. `feature/login`.
    pub incoming: String,
}

/// A decision about how a single conflict should be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Keep the current ("ours") side.
    Current,
    /// Keep the incoming ("theirs") side.
    Incoming,
    /// Keep the common ancestor (only meaningful with a base present).
    Base,
    /// Keep both sides concatenated.
    Both { incoming_first: bool },
    /// Replace the region with hand-edited lines.
    Custom(Vec<String>),
}

/// A single conflict region together with its (optional) chosen resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub current: Vec<String>,
    pub base: Option<Vec<String>>,
    pub incoming: Vec<String>,
    pub labels: ConflictLabels,
    pub resolution: Option<Resolution>,
}

impl Conflict {
    /// Whether this conflict has diff3-style base content available.
    pub fn has_base(&self) -> bool {
        self.base.is_some()
    }

    /// Whether a resolution has been chosen.
    pub fn is_resolved(&self) -> bool {
        self.resolution.is_some()
    }

    /// The lines for a given side.
    pub fn side(&self, side: Side) -> &[String] {
        match side {
            Side::Current => &self.current,
            Side::Base => self.base.as_deref().unwrap_or(&[]),
            Side::Incoming => &self.incoming,
        }
    }

    /// Attach (or replace) a resolution.
    pub fn resolve(&mut self, resolution: Resolution) {
        self.resolution = Some(resolution);
    }

    /// Drop any chosen resolution, returning the conflict to "unresolved".
    pub fn clear(&mut self) {
        self.resolution = None;
    }

    /// The lines this conflict contributes once resolved, or `None` while it is
    /// still outstanding.
    pub fn resolved_lines(&self) -> Option<Vec<String>> {
        let resolution = self.resolution.as_ref()?;
        Some(match resolution {
            Resolution::Current => self.current.clone(),
            Resolution::Incoming => self.incoming.clone(),
            Resolution::Base => self.base.clone().unwrap_or_default(),
            Resolution::Both { incoming_first } => {
                let mut lines = Vec::with_capacity(self.current.len() + self.incoming.len());
                if *incoming_first {
                    lines.extend(self.incoming.iter().cloned());
                    lines.extend(self.current.iter().cloned());
                } else {
                    lines.extend(self.current.iter().cloned());
                    lines.extend(self.incoming.iter().cloned());
                }
                lines
            }
            Resolution::Custom(lines) => lines.clone(),
        })
    }

    /// Render this conflict back into raw lines, including markers, exactly as it
    /// would appear in a conflicted file. Used by [`MergeDocument::render`] for
    /// conflicts that have not been resolved.
    fn render_unresolved(&self, out: &mut Vec<String>) {
        out.push(marker_line('<', &self.labels.current));
        out.extend(self.current.iter().cloned());
        if let Some(base) = &self.base {
            out.push(marker_line('|', self.labels.base.as_deref().unwrap_or("")));
            out.extend(base.iter().cloned());
        }
        out.push(marker_line('=', ""));
        out.extend(self.incoming.iter().cloned());
        out.push(marker_line('>', &self.labels.incoming));
    }
}

/// One run of a parsed file: either non-conflicted text or a conflict region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    Stable(Vec<String>),
    Conflict(Conflict),
}

/// A parsed conflicted file: an ordered list of sections plus the formatting
/// details needed to render it back losslessly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeDocument {
    pub sections: Vec<Section>,
    pub line_ending: LineEnding,
    pub trailing_newline: bool,
}

impl MergeDocument {
    /// Parse file contents into a structured document. This never fails: input
    /// without any conflict markers parses to a single stable section, and
    /// malformed/unterminated markers are absorbed into the surrounding text.
    pub fn parse(content: &str) -> Self {
        let line_ending = if content.contains("\r\n") {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        };
        let trailing_newline = content.ends_with('\n');

        let normalized = content.replace("\r\n", "\n");
        let mut raw: Vec<&str> = normalized.split('\n').collect();
        // `split` on a trailing newline yields a final empty element we don't
        // want to treat as a real line.
        if trailing_newline {
            raw.pop();
        }

        let sections = parse_sections(&raw);

        MergeDocument {
            sections,
            line_ending,
            trailing_newline,
        }
    }

    /// Iterate over the conflicts in document order.
    pub fn conflicts(&self) -> impl Iterator<Item = &Conflict> {
        self.sections.iter().filter_map(|s| match s {
            Section::Conflict(c) => Some(c),
            Section::Stable(_) => None,
        })
    }

    /// Mutably iterate over the conflicts in document order.
    pub fn conflicts_mut(&mut self) -> impl Iterator<Item = &mut Conflict> {
        self.sections.iter_mut().filter_map(|s| match s {
            Section::Conflict(c) => Some(c),
            Section::Stable(_) => None,
        })
    }

    /// Borrow the `n`th conflict (0-based, document order).
    pub fn conflict(&self, n: usize) -> Option<&Conflict> {
        self.conflicts().nth(n)
    }

    /// Mutably borrow the `n`th conflict (0-based, document order).
    pub fn conflict_mut(&mut self, n: usize) -> Option<&mut Conflict> {
        self.conflicts_mut().nth(n)
    }

    /// Total number of conflicts in the document.
    pub fn conflict_count(&self) -> usize {
        self.conflicts().count()
    }

    /// Number of conflicts that have a resolution attached.
    pub fn resolved_count(&self) -> usize {
        self.conflicts().filter(|c| c.is_resolved()).count()
    }

    /// Number of conflicts still awaiting a decision.
    pub fn unresolved_count(&self) -> usize {
        self.conflict_count() - self.resolved_count()
    }

    /// Whether every conflict has been resolved. A file with no conflicts at all
    /// is trivially fully resolved.
    pub fn is_fully_resolved(&self) -> bool {
        self.unresolved_count() == 0
    }

    /// Convenience: resolve the `n`th conflict. Returns `false` if out of range.
    pub fn resolve(&mut self, n: usize, resolution: Resolution) -> bool {
        match self.conflict_mut(n) {
            Some(c) => {
                c.resolve(resolution);
                true
            }
            None => false,
        }
    }

    /// Render the document back to text. Resolved conflicts emit their chosen
    /// side; unresolved conflicts re-emit their original markers. Round-trips a
    /// fully-unresolved document byte-for-byte (modulo a missing trailing
    /// newline being preserved as missing).
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for section in &self.sections {
            match section {
                Section::Stable(stable) => lines.extend(stable.iter().cloned()),
                Section::Conflict(conflict) => match conflict.resolved_lines() {
                    Some(resolved) => lines.extend(resolved),
                    None => conflict.render_unresolved(&mut lines),
                },
            }
        }

        let mut out = lines.join(self.line_ending.as_str());
        // Re-attach the trailing terminator the join dropped — but only when
        // there is actually content. A document that resolves to zero lines is
        // an empty file, not a file containing a single blank line.
        if self.trailing_newline && !lines.is_empty() {
            out.push_str(self.line_ending.as_str());
        }
        out
    }
}

impl fmt::Display for MergeDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// State machine over the raw lines of a file, splitting it into sections.
fn parse_sections(raw: &[&str]) -> Vec<Section> {
    enum State {
        Stable,
        Current,
        Base,
        Incoming,
    }

    let mut sections: Vec<Section> = Vec::new();
    let mut state = State::Stable;

    let mut stable: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut base: Vec<String> = Vec::new();
    let mut incoming: Vec<String> = Vec::new();
    let mut labels = ConflictLabels::default();
    let mut saw_base = false;

    let flush_stable = |stable: &mut Vec<String>, sections: &mut Vec<Section>| {
        if !stable.is_empty() {
            sections.push(Section::Stable(std::mem::take(stable)));
        }
    };

    for &line in raw {
        match state {
            State::Stable => {
                if let Some(label) = marker_label(line, '<') {
                    flush_stable(&mut stable, &mut sections);
                    labels = ConflictLabels::default();
                    labels.current = label.to_string();
                    current.clear();
                    base.clear();
                    incoming.clear();
                    saw_base = false;
                    state = State::Current;
                } else {
                    stable.push(line.to_string());
                }
            }
            State::Current => {
                if let Some(label) = marker_label(line, '|') {
                    labels.base = Some(label.to_string());
                    saw_base = true;
                    state = State::Base;
                } else if marker_label(line, '=').is_some() {
                    state = State::Incoming;
                } else {
                    current.push(line.to_string());
                }
            }
            State::Base => {
                if marker_label(line, '=').is_some() {
                    state = State::Incoming;
                } else {
                    base.push(line.to_string());
                }
            }
            State::Incoming => {
                if let Some(label) = marker_label(line, '>') {
                    labels.incoming = label.to_string();
                    sections.push(Section::Conflict(Conflict {
                        current: std::mem::take(&mut current),
                        base: if saw_base {
                            Some(std::mem::take(&mut base))
                        } else {
                            None
                        },
                        incoming: std::mem::take(&mut incoming),
                        labels: std::mem::take(&mut labels),
                        resolution: None,
                    }));
                    state = State::Stable;
                } else {
                    incoming.push(line.to_string());
                }
            }
        }
    }

    // An unterminated conflict (no closing `>>>>>>>`) is not a real conflict —
    // fall back to treating the collected lines, markers included, as stable
    // text so we never lose content.
    if !matches!(state, State::Stable) {
        let mut leftover: Vec<String> = std::mem::take(&mut stable);
        leftover.push(marker_line('<', &labels.current));
        leftover.append(&mut current);
        if saw_base {
            leftover.push(marker_line('|', labels.base.as_deref().unwrap_or("")));
            leftover.append(&mut base);
        }
        if matches!(state, State::Incoming) {
            leftover.push(marker_line('=', ""));
            leftover.append(&mut incoming);
        }
        // Merge into the preceding stable run if there is one, so an unterminated
        // region doesn't fragment the surrounding text into two sections.
        match sections.last_mut() {
            Some(Section::Stable(existing)) => existing.extend(leftover),
            _ => sections.push(Section::Stable(leftover)),
        }
    } else {
        flush_stable(&mut stable, &mut sections);
    }

    sections
}

/// If `line` is a conflict marker built from `marker_char` (a run of at least
/// [`MARKER_LEN`] of that char, then end-of-line or a space + label), return the
/// trailing label (trimmed, possibly empty). Otherwise `None`.
fn marker_label(line: &str, marker_char: char) -> Option<&str> {
    let bytes = line.as_bytes();
    let needle = marker_char as u8;
    let run = bytes.iter().take_while(|&&b| b == needle).count();
    if run < MARKER_LEN {
        return None;
    }
    match bytes.get(run) {
        None => Some(""),
        Some(&b' ') => Some(line[run + 1..].trim_end()),
        Some(_) => None,
    }
}

/// Build a marker line: a run of `MARKER_LEN` marker chars, plus `" label"` when
/// a non-empty label is supplied.
fn marker_line(marker_char: char, label: &str) -> String {
    let mut s: String = std::iter::repeat(marker_char).take(MARKER_LEN).collect();
    if !label.is_empty() {
        s.push(' ');
        s.push_str(label);
    }
    s
}
