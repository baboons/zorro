//! Token-level diffing for highlighting *within* a conflict.
//!
//! The conflict engine works line-by-line, but a good merge UI also highlights
//! which words or characters actually changed between two versions of a line.
//! This module provides a small, dependency-free LCS diff over tokens, plus
//! tokenizers for word- and character-level granularity.

/// One span of a diff result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffTag {
    /// Present in both sides, unchanged.
    Equal,
    /// Present only on the left (removed).
    Delete,
    /// Present only on the right (added).
    Insert,
}

/// A contiguous run of tokens sharing a [`DiffTag`], rejoined into a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSpan {
    pub tag: DiffTag,
    pub text: String,
}

/// Granularity for [`diff`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    /// Each character (well, Unicode scalar) is a token.
    Char,
    /// Words and the whitespace/punctuation between them are tokens, so edits
    /// land on whole-word boundaries.
    Word,
}

/// Diff `left` against `right` at the requested granularity, returning a
/// coalesced list of spans. Adjacent tokens with the same tag are merged so the
/// result is convenient to render directly.
pub fn diff(left: &str, right: &str, granularity: Granularity) -> Vec<DiffSpan> {
    let lt = tokenize(left, granularity);
    let rt = tokenize(right, granularity);
    let ops = lcs_diff(&lt, &rt);
    coalesce(ops)
}

/// Line-level diff between two blocks (e.g. the two sides of a conflict).
///
/// Returns `(removed, added)`: `removed[i]` marks a left line absent on the
/// right (rendered red), `added[j]` a right line absent on the left (rendered
/// green). Lines common to both are `false` in both vectors.
pub fn line_diff_flags(left: &[String], right: &[String]) -> (Vec<bool>, Vec<bool>) {
    let l: Vec<&str> = left.iter().map(String::as_str).collect();
    let r: Vec<&str> = right.iter().map(String::as_str).collect();
    let ops = lcs_diff(&l, &r);

    let mut removed = vec![false; left.len()];
    let mut added = vec![false; right.len()];
    let (mut i, mut j) = (0usize, 0usize);
    for (tag, _) in ops {
        match tag {
            DiffTag::Equal => {
                i += 1;
                j += 1;
            }
            DiffTag::Delete => {
                if i < removed.len() {
                    removed[i] = true;
                }
                i += 1;
            }
            DiffTag::Insert => {
                if j < added.len() {
                    added[j] = true;
                }
                j += 1;
            }
        }
    }
    (removed, added)
}

fn tokenize(s: &str, granularity: Granularity) -> Vec<&str> {
    match granularity {
        Granularity::Char => {
            let mut out = Vec::new();
            let mut indices = s.char_indices().peekable();
            while let Some((start, _)) = indices.next() {
                let end = indices.peek().map(|&(i, _)| i).unwrap_or(s.len());
                out.push(&s[start..end]);
            }
            out
        }
        Granularity::Word => {
            // Group runs of "word characters" (alphanumeric + `_`) together, and
            // emit every other character as its own token. This keeps words
            // intact while letting spaces and punctuation diff individually.
            let mut out = Vec::new();
            let mut start = 0usize;
            let mut in_word = false;
            for (i, ch) in s.char_indices() {
                let is_word = ch.is_alphanumeric() || ch == '_';
                if i == 0 {
                    in_word = is_word;
                    start = 0;
                    continue;
                }
                if is_word != in_word {
                    out.push(&s[start..i]);
                    start = i;
                    in_word = is_word;
                }
            }
            if start < s.len() {
                out.push(&s[start..]);
            }
            out
        }
    }
}

/// Classic dynamic-programming LCS, walked back into a token-level op list.
fn lcs_diff<'a>(left: &[&'a str], right: &[&'a str]) -> Vec<(DiffTag, &'a str)> {
    let n = left.len();
    let m = right.len();

    // table[i][j] = LCS length of left[i..] and right[j..].
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if left[i] == right[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if left[i] == right[j] {
            ops.push((DiffTag::Equal, left[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push((DiffTag::Delete, left[i]));
            i += 1;
        } else {
            ops.push((DiffTag::Insert, right[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push((DiffTag::Delete, left[i]));
        i += 1;
    }
    while j < m {
        ops.push((DiffTag::Insert, right[j]));
        j += 1;
    }
    ops
}

fn coalesce(ops: Vec<(DiffTag, &str)>) -> Vec<DiffSpan> {
    let mut spans: Vec<DiffSpan> = Vec::new();
    for (tag, text) in ops {
        match spans.last_mut() {
            Some(last) if last.tag == tag => last.text.push_str(text),
            _ => spans.push(DiffSpan {
                tag,
                text: text.to_string(),
            }),
        }
    }
    spans
}
