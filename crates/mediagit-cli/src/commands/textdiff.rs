// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Line-by-line changes for `diff -p`, for text files only.
//!
//! Most files in a media repository are binary, so this is opt-in and never
//! runs on a default path. A side qualifies as text only when it is small
//! enough to read (not chunked, at most [`MAX_SIDE`] bytes), has no NUL in its
//! first 8000 bytes, and is valid UTF-8. Everything else is reported as
//! `binary` or `too_large` with no hunks.

use serde::Serialize;
use similar::{ChangeTag, TextDiff};

/// Largest side read for a line diff. Same cutoff as `diff`'s binary sniff.
pub const MAX_SIDE: u64 = 8 * 1024 * 1024;
/// Output lines per file before the hunks are cut off (`truncated`).
pub const MAX_LINES: usize = 5000;
const SNIFF: usize = 8000;
const CONTEXT: usize = 3;
/// Upper bound on line-diff time. Past it the diff is coarser, never wrong.
const DIFF_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// `diff -p` result for one file.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct FileText {
    /// `text` (hunks below), `binary` or `too_large` (no hunks).
    pub kind: String,
    pub hunks: Vec<DiffHunk>,
    /// More than [`MAX_LINES`] lines changed: the rest is left out.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct DiffHunk {
    /// 1-based first line on each side (as in `@@ -a,b +c,d @@`).
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct DiffLine {
    /// `+` added, `-` removed, ` ` unchanged context.
    pub kind: String,
    /// The line without its line ending.
    pub text: String,
}

/// One side of a comparison, as loaded by the caller.
pub enum Side {
    /// The file does not exist on this side (added or deleted).
    Absent,
    /// Small enough to compare: the bytes.
    Bytes(Vec<u8>),
    /// Chunked or above [`MAX_SIDE`]: not read.
    TooLarge,
}

fn verdict(kind: &str) -> FileText {
    FileText {
        kind: kind.into(),
        hunks: Vec::new(),
        truncated: false,
    }
}

/// Text of one side, `Err(kind)` when it does not qualify.
fn as_text(side: &Side) -> Result<&str, &'static str> {
    match side {
        Side::Absent => Ok(""),
        Side::TooLarge => Err("too_large"),
        Side::Bytes(b) if b.iter().take(SNIFF).any(|&c| c == 0) => Err("binary"),
        Side::Bytes(b) => std::str::from_utf8(b).map_err(|_| "binary"),
    }
}

/// Line-by-line changes from `old` to `new`.
pub fn compare(old: &Side, new: &Side) -> FileText {
    let (old, new) = match (as_text(old), as_text(new)) {
        (Err("too_large"), _) | (_, Err("too_large")) => return verdict("too_large"),
        (Err(k), _) | (_, Err(k)) => return verdict(k),
        (Ok(o), Ok(n)) => (o, n),
    };
    // Myers diff slows down sharply when most lines change (a re-exported
    // .obj or .usda); past the deadline `similar` returns a coarser diff.
    let diff = TextDiff::configure()
        .timeout(DIFF_DEADLINE)
        .diff_lines(old, new);
    let mut out = verdict("text");
    let mut emitted = 0usize;
    for group in diff.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let (o, n) = (first.old_range().start, first.new_range().start);
        let (o_len, n_len) = (last.old_range().end - o, last.new_range().end - n);
        let mut hunk = DiffHunk {
            // An empty side starts at 0, as in unified diffs.
            old_start: (if o_len == 0 { o } else { o + 1 }) as u32,
            old_lines: o_len as u32,
            new_start: (if n_len == 0 { n } else { n + 1 }) as u32,
            new_lines: n_len as u32,
            lines: Vec::new(),
        };
        for op in &group {
            for change in diff.iter_changes(op) {
                if emitted == MAX_LINES {
                    out.truncated = true;
                    if !hunk.lines.is_empty() {
                        // The header must count only the lines actually kept.
                        let count =
                            |k: &str| hunk.lines.iter().filter(|l| l.kind != k).count() as u32;
                        hunk.old_lines = count("+");
                        hunk.new_lines = count("-");
                        // A side with nothing kept starts at 0, as in unified diffs.
                        if hunk.old_lines == 0 && o_len > 0 {
                            hunk.old_start -= 1;
                        }
                        if hunk.new_lines == 0 && n_len > 0 {
                            hunk.new_start -= 1;
                        }
                        out.hunks.push(hunk);
                    }
                    return out;
                }
                let kind = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                let text = change.value();
                let text = text.strip_suffix('\n').unwrap_or(text);
                let text = text.strip_suffix('\r').unwrap_or(text);
                hunk.lines.push(DiffLine {
                    kind: kind.into(),
                    text: text.to_owned(),
                });
                emitted += 1;
            }
        }
        out.hunks.push(hunk);
    }
    out
}

/// Unified-diff text for a terminal (`diff -p` without `--machine`).
pub fn render(path: &str, t: &FileText) -> String {
    let mut s = String::new();
    match t.kind.as_str() {
        "binary" => s.push_str(&format!("{path}: binary file, no line-by-line changes\n")),
        "too_large" => s.push_str(&format!(
            "{path}: too large to compare line by line (over {} MiB)\n",
            MAX_SIDE / (1024 * 1024)
        )),
        _ => {
            s.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
            for h in &t.hunks {
                s.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    h.old_start, h.old_lines, h.new_start, h.new_lines
                ));
                for l in &h.lines {
                    s.push_str(&l.kind);
                    s.push_str(&l.text);
                    s.push('\n');
                }
            }
            if t.truncated {
                s.push_str(&format!(
                    "... more than {MAX_LINES} lines changed; the rest is not shown\n"
                ));
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Side {
        Side::Bytes(s.as_bytes().to_vec())
    }
    fn lines(t: &FileText) -> Vec<String> {
        t.hunks
            .iter()
            .flat_map(|h| h.lines.iter().map(|l| format!("{}{}", l.kind, l.text)))
            .collect()
    }

    #[test]
    fn a_changed_line_is_one_removal_and_one_addition_with_context() {
        let t = compare(&b("a\nb\nc\n"), &b("a\nB\nc\n"));
        assert_eq!(t.kind, "text");
        assert_eq!(lines(&t), [" a", "-b", "+B", " c"]);
        let h = &t.hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (1, 3, 1, 3)
        );
    }

    #[test]
    fn added_and_deleted_files_are_all_plus_or_all_minus() {
        let added = compare(&Side::Absent, &b("x\ny\n"));
        assert_eq!(lines(&added), ["+x", "+y"]);
        assert_eq!((added.hunks[0].old_start, added.hunks[0].old_lines), (0, 0));
        let deleted = compare(&b("x\n"), &Side::Absent);
        assert_eq!(lines(&deleted), ["-x"]);
    }

    #[test]
    fn crlf_and_a_missing_final_newline_do_not_leak_into_the_text() {
        let t = compare(&b("a\r\nb\r\n"), &b("a\r\nc"));
        assert_eq!(lines(&t), [" a", "-b", "+c"]);
    }

    #[test]
    fn far_apart_changes_make_separate_hunks() {
        let line = |i: i32| match i {
            2 => "two\n".to_owned(),
            29 => "twenty-nine\n".to_owned(),
            _ => format!("{i}\n"),
        };
        let old: String = (1..=30).map(|i| format!("{i}\n")).collect();
        let new: String = (1..=30).map(line).collect();
        let t = compare(&b(&old), &b(&new));
        assert_eq!(t.hunks.len(), 2);
    }

    #[test]
    fn binary_invalid_utf8_and_too_large_get_no_hunks() {
        assert_eq!(compare(&b("a\0b"), &b("a")).kind, "binary");
        assert_eq!(
            compare(&Side::Bytes(vec![0xff, 0xfe, b'a']), &b("a")).kind,
            "binary"
        );
        assert_eq!(compare(&Side::TooLarge, &b("a")).kind, "too_large");
        assert_eq!(compare(&b("a"), &Side::TooLarge).kind, "too_large");
        assert!(compare(&Side::TooLarge, &b("a")).hunks.is_empty());
    }

    #[test]
    fn huge_changes_are_cut_off_and_flagged() {
        let new: String = (0..MAX_LINES + 50).map(|i| format!("{i}\n")).collect();
        let t = compare(&Side::Absent, &b(&new));
        assert!(t.truncated);
        assert_eq!(lines(&t).len(), MAX_LINES);
    }

    #[test]
    fn render_prints_a_unified_diff() {
        let t = compare(&b("a\n"), &b("b\n"));
        let s = render("x.txt", &t);
        assert!(
            s.contains("--- a/x.txt\n+++ b/x.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n"),
            "{s}"
        );
        assert!(render("y.psd", &verdict("binary")).contains("binary file"));
    }

    #[test]
    fn a_cut_hunk_header_counts_only_the_kept_lines() {
        let old: String = (0..MAX_LINES).map(|i| format!("old{i}\n")).collect();
        let new: String = (0..MAX_LINES).map(|i| format!("new{i}\n")).collect();
        let t = compare(&b(&old), &b(&new));
        assert!(t.truncated);
        let h = t.hunks.last().unwrap();
        let minus = h.lines.iter().filter(|l| l.kind == "-").count() as u32;
        let plus = h.lines.iter().filter(|l| l.kind == "+").count() as u32;
        assert_eq!((h.old_lines, h.new_lines), (minus, plus));
        // Every kept line is a removal: the new side is empty and starts at 0.
        assert_eq!((h.new_start, h.new_lines), (0, 0));
    }

    #[test]
    fn a_file_where_every_line_changes_finishes_quickly() {
        // Interleaved changes are Myers' worst case; the deadline bounds it.
        let old: String = (0..200_000).map(|i| format!("{i}\n")).collect();
        let new: String = (0..200_000).map(|i| format!("{}\n", i * 7 + 3)).collect();
        let t0 = std::time::Instant::now();
        let t = compare(&b(&old), &b(&new));
        assert_eq!(t.kind, "text");
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(15),
            "{:?}",
            t0.elapsed()
        );
    }
}
