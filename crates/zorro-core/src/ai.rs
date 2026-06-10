//! AI-assisted conflict resolution.
//!
//! This module is provider-agnostic and dependency-free: it defines the request
//! Zorro hands to an AI ([`AiConflict`]), the structured answer it expects back
//! ([`Suggestion`]), the prompt construction, the response parsing, and a
//! confidence heuristic. Concrete providers implement [`AiProvider`]; the only
//! one here is [`CliProvider`], which shells out to a CLI such as
//! `claude -p`, `codex`, or `ollama run`.
//!
//! Nothing is ever applied automatically — the app shows the suggestion and the
//! user approves it. The provider methods are synchronous (they block on a
//! subprocess); the UI runs them on a background executor.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::syntax::Language;

/// How sure the AI (or our heuristic) is about a suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "High",
            Confidence::Medium => "Medium",
            Confidence::Low => "Low",
        }
    }

    /// Parse a `Confidence: High` style marker (case-insensitive).
    fn parse(word: &str) -> Option<Confidence> {
        match word.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Confidence::High),
            "medium" | "med" => Some(Confidence::Medium),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
}

/// Everything an AI needs to reason about a single conflict.
#[derive(Clone, Debug)]
pub struct AiConflict {
    pub path: String,
    pub language: Language,
    pub base: Option<Vec<String>>,
    pub current: Vec<String>,
    pub incoming: Vec<String>,
    /// A few unchanged lines immediately before / after the conflict.
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

/// A proposed resolution: the merged code plus a confidence and any prose notes
/// the model returned alongside the code block.
#[derive(Clone, Debug)]
pub struct Suggestion {
    pub code: String,
    pub confidence: Confidence,
    pub notes: Option<String>,
}

/// What can go wrong talking to a provider.
#[derive(Debug)]
pub enum AiError {
    /// The provider's CLI isn't installed / on PATH.
    CliMissing(String),
    /// Failed to spawn or talk to the process.
    Spawn(std::io::Error),
    /// The process ran but exited non-zero.
    Failed(String),
    /// The process produced no usable output.
    Empty,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::CliMissing(name) => write!(f, "{name} is not installed or not on PATH"),
            AiError::Spawn(e) => write!(f, "could not run provider: {e}"),
            AiError::Failed(msg) => write!(f, "provider failed: {}", msg.trim()),
            AiError::Empty => write!(f, "provider returned no output"),
        }
    }
}

impl std::error::Error for AiError {}

/// A pluggable AI backend. Methods block; the caller runs them off the UI thread.
pub trait AiProvider: Send + Sync {
    /// Human-readable provider name (shown in the UI).
    fn name(&self) -> &str;
    /// Explain why the conflict exists and how it should be merged (prose).
    fn explain(&self, conflict: &AiConflict) -> Result<String, AiError>;
    /// Propose a merged resolution.
    fn resolve(&self, conflict: &AiConflict) -> Result<Suggestion, AiError>;
}

// ---- prompts ---------------------------------------------------------------

fn fenced(lines: &[String]) -> String {
    if lines.is_empty() {
        "(empty)".to_string()
    } else {
        lines.join("\n")
    }
}

/// Build the resolve prompt. The model is asked to return a confidence marker
/// followed by a single fenced code block containing only the merged code.
pub fn resolve_prompt(c: &AiConflict) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "You are resolving a Git merge conflict in `{}` ({:?}).\n\n",
        c.path, c.language
    ));
    p.push_str(
        "Combine the intent of BOTH sides into correct, compilable code. \
         Respond with exactly:\n\
         - a line `Confidence: High|Medium|Low`\n\
         - then a single fenced code block containing ONLY the merged lines that \
         replace the conflict (no surrounding context, no explanation).\n\n",
    );
    if !c.context_before.is_empty() {
        p.push_str("CONTEXT BEFORE:\n```\n");
        p.push_str(&fenced(&c.context_before));
        p.push_str("\n```\n\n");
    }
    if let Some(base) = &c.base {
        p.push_str("BASE (common ancestor):\n```\n");
        p.push_str(&fenced(base));
        p.push_str("\n```\n\n");
    }
    p.push_str("CURRENT (ours):\n```\n");
    p.push_str(&fenced(&c.current));
    p.push_str("\n```\n\nINCOMING (theirs):\n```\n");
    p.push_str(&fenced(&c.incoming));
    p.push_str("\n```\n");
    if !c.context_after.is_empty() {
        p.push_str("\nCONTEXT AFTER:\n```\n");
        p.push_str(&fenced(&c.context_after));
        p.push_str("\n```\n");
    }
    p
}

/// Build the explain prompt (free-form prose answer).
pub fn explain_prompt(c: &AiConflict) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "Explain this Git merge conflict in `{}` concisely.\n\n",
        c.path
    ));
    p.push_str(
        "Cover: what the current branch changed, what the incoming branch changed, \
         why they conflict, and the recommended merge. Keep it short.\n\n",
    );
    if let Some(base) = &c.base {
        p.push_str("BASE:\n```\n");
        p.push_str(&fenced(base));
        p.push_str("\n```\n\n");
    }
    p.push_str("CURRENT:\n```\n");
    p.push_str(&fenced(&c.current));
    p.push_str("\n```\n\nINCOMING:\n```\n");
    p.push_str(&fenced(&c.incoming));
    p.push_str("\n```\n");
    p
}

// ---- response parsing ------------------------------------------------------

/// Parse a resolve response into a [`Suggestion`]: pull the confidence marker and
/// the first fenced code block. Falls back to `fallback` confidence and, if there
/// is no code fence, to the whole trimmed response as the code.
pub fn parse_suggestion(raw: &str, fallback: Confidence) -> Suggestion {
    let confidence = raw
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .trim()
                .strip_prefix("confidence:")
                .and_then(Confidence::parse)
        })
        .unwrap_or(fallback);

    let (code, notes) = match extract_code_block(raw) {
        Some(code) => {
            let notes = prose_outside_block(raw);
            (code, notes)
        }
        None => (raw.trim().to_string(), None),
    };

    Suggestion {
        code,
        confidence,
        notes,
    }
}

/// Extract the contents of the first fenced (```) code block.
fn extract_code_block(raw: &str) -> Option<String> {
    let mut lines = raw.lines();
    let mut body: Vec<&str> = Vec::new();
    // Find the opening fence.
    for line in lines.by_ref() {
        if line.trim_start().starts_with("```") {
            break;
        }
    }
    let mut closed = false;
    for line in lines {
        if line.trim_start().starts_with("```") {
            closed = true;
            break;
        }
        body.push(line);
    }
    if closed {
        Some(body.join("\n"))
    } else {
        None
    }
}

/// Any non-fence prose (the `Confidence:` line is dropped) before the code block.
fn prose_outside_block(raw: &str) -> Option<String> {
    let mut prose: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            break;
        }
        if trimmed.to_ascii_lowercase().starts_with("confidence:") {
            continue;
        }
        if !line.trim().is_empty() {
            prose.push(line);
        }
    }
    if prose.is_empty() {
        None
    } else {
        Some(prose.join("\n"))
    }
}

// ---- confidence heuristic --------------------------------------------------

/// A cheap, model-free confidence estimate used as a fallback (and for the
/// "resolve trivial conflicts" pass): import-only / tiny changes are High, small
/// changes Medium, large ones Low.
pub fn heuristic_confidence(c: &AiConflict) -> Confidence {
    let changed = c.current.len() + c.incoming.len();

    let only_imports = c
        .current
        .iter()
        .chain(c.incoming.iter())
        .filter(|l| !l.trim().is_empty())
        .all(|l| is_import_line(l));
    if only_imports {
        return Confidence::High;
    }

    if changed <= 2 {
        Confidence::High
    } else if changed <= 8 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

fn is_import_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("import ")
        || t.starts_with("use ")
        || t.starts_with("#include")
        || t.starts_with("from ")
        || t.starts_with("require(")
}

// ---- CLI provider ----------------------------------------------------------

/// A provider backed by a command-line tool that reads a prompt on stdin and
/// prints its answer to stdout (e.g. `claude -p`, `codex`, `ollama run llama3`).
#[derive(Clone, Debug)]
pub struct CliProvider {
    label: String,
    program: String,
    args: Vec<String>,
}

impl CliProvider {
    pub fn new(
        label: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> CliProvider {
        CliProvider {
            label: label.into(),
            program: program.into(),
            args,
        }
    }

    /// Claude Code in non-interactive print mode.
    pub fn claude_code() -> CliProvider {
        CliProvider::new("Claude Code", "claude", vec!["-p".into()])
    }

    /// Codex CLI.
    pub fn codex() -> CliProvider {
        CliProvider::new("Codex CLI", "codex", vec!["exec".into()])
    }

    /// A local Ollama model.
    pub fn ollama(model: impl Into<String>) -> CliProvider {
        CliProvider::new("Ollama", "ollama", vec!["run".into(), model.into()])
    }

    fn run(&self, prompt: &str) -> Result<String, AiError> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AiError::CliMissing(self.program.clone())
                } else {
                    AiError::Spawn(e)
                }
            })?;

        // Write the prompt and close stdin so the tool starts processing.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).map_err(AiError::Spawn)?;
        }

        let output = child.wait_with_output().map_err(AiError::Spawn)?;
        if !output.status.success() {
            return Err(AiError::Failed(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if text.trim().is_empty() {
            return Err(AiError::Empty);
        }
        Ok(text)
    }
}

impl AiProvider for CliProvider {
    fn name(&self) -> &str {
        &self.label
    }

    fn explain(&self, conflict: &AiConflict) -> Result<String, AiError> {
        Ok(self.run(&explain_prompt(conflict))?.trim().to_string())
    }

    fn resolve(&self, conflict: &AiConflict) -> Result<Suggestion, AiError> {
        let raw = self.run(&resolve_prompt(conflict))?;
        Ok(parse_suggestion(&raw, heuristic_confidence(conflict)))
    }
}
