//! Heuristic text block classifier.
//!
//! Examines the first bytes of a raw text block and returns a `(TextKind,
//! confidence, extension)` triple.  The classifier is intentionally simple and
//! fast — it is called for every block emitted by the scanner.

use crate::scanner::TextKind;

// ── Public interface ──────────────────────────────────────────────────────────

/// Classify the content of a text block.
///
/// `data` should be the raw bytes of the block.  The classifier only examines
/// the first 1 KiB.
///
/// Returns `(kind, confidence 0–100, extension)`.  The `extension` may differ
/// from `kind.extension()` for `Script` blocks whose shebang was sub-classified.
pub fn classify(data: &[u8]) -> (TextKind, u8, &'static str) {
    let head = &data[..data.len().min(1024)];

    // ── Priority-ordered checks ───────────────────────────────────────────────

    // PHP opener
    if head.starts_with(b"<?php") && head.len() > 5 && is_ws(head[5]) {
        return (TextKind::Php, 99, "php");
    }

    // Unix shebang
    if head.starts_with(b"#!") && head.len() > 2 && head[2] == b'/' {
        let ext = classify_shebang(head);
        return (TextKind::Script, 97, ext);
    }

    // XML
    if head.starts_with(b"<?xml") {
        return (TextKind::Markup, 99, "xml");
    }

    // HTML
    if starts_with_ci(head, b"<!DOCTYPE")
        || starts_with_ci(head, b"<html")
        || starts_with_ci(head, b"<HTML")
    {
        return (TextKind::Markup, 95, "html");
    }

    // JSON — starts with { or [ and has at least 2 "key": patterns
    if (head.first() == Some(&b'{') || head.first() == Some(&b'['))
        && count_json_key_patterns(head) >= 2
    {
        return (TextKind::Json, 85, "json");
    }

    // YAML frontmatter
    if head.starts_with(b"---\n") || head.starts_with(b"---\r\n") {
        return (TextKind::Yaml, 90, "yaml");
    }

    // SQL keyword density
    if count_distinct_sql_keywords(head) >= 3 {
        return (TextKind::Sql, 70, "sql");
    }

    // C/C++ keyword density
    if count_distinct_c_keywords(head) >= 3 {
        return (TextKind::CSource, 65, "c");
    }

    // Markdown density
    if count_markdown_signals(head) >= 2 {
        return (TextKind::Markdown, 65, "md");
    }

    (TextKind::Generic, 50, "txt")
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[inline]
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Case-insensitive `starts_with` for a fixed ASCII needle.
fn starts_with_ci(data: &[u8], needle: &[u8]) -> bool {
    if data.len() < needle.len() {
        return false;
    }
    data[..needle.len()]
        .iter()
        .zip(needle)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Classify a shebang line to determine the script type.
///
/// Reads from `#!` onwards; returns one of `"py"`, `"rb"`, `"pl"`, `"js"`, `"sh"`.
fn classify_shebang(data: &[u8]) -> &'static str {
    // Extract first line (up to LF or end of data).
    let line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    let line = &data[..line_end];
    if contains_subslice(line, b"python") {
        "py"
    } else if contains_subslice(line, b"ruby") {
        "rb"
    } else if contains_subslice(line, b"perl") {
        "pl"
    } else if contains_subslice(line, b"node") {
        "js"
    } else {
        "sh"
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Count occurrences of the JSON key pattern `":` (quote followed by colon)
/// in the first 1 KiB.  Every JSON key-value pair produces at least one.
fn count_json_key_patterns(data: &[u8]) -> usize {
    data.windows(2).filter(|w| w[0] == b'"' && w[1] == b':').count()
}

/// Count distinct SQL keywords present in `data` (case-insensitive).
fn count_distinct_sql_keywords(data: &[u8]) -> usize {
    const KWS: &[&[u8]] = &[
        b"SELECT", b"INSERT", b"CREATE", b"UPDATE", b"DELETE", b"FROM", b"WHERE",
    ];
    KWS.iter()
        .filter(|&&kw| contains_keyword_ci(data, kw))
        .count()
}

/// Count distinct C/C++ keywords/directives present in `data`.
fn count_distinct_c_keywords(data: &[u8]) -> usize {
    const KWS: &[&[u8]] = &[
        b"#include", b"#define", b"typedef", b"struct", b"void", b"int",
    ];
    KWS.iter()
        .filter(|&&kw| contains_subslice(data, kw))
        .count()
}

/// Count Markdown structural signals in `data`.
fn count_markdown_signals(data: &[u8]) -> usize {
    let mut count = 0;
    // ATX headings: "# " or "## "
    if contains_subslice(data, b"# ") || contains_subslice(data, b"## ") {
        count += 1;
    }
    // Bold: **
    if contains_subslice(data, b"**") {
        count += 1;
    }
    // Inline link: ](
    if contains_subslice(data, b"](") {
        count += 1;
    }
    // Task list: - [ ] or - [x]
    if contains_subslice(data, b"- [") {
        count += 1;
    }
    count
}

/// Case-insensitive keyword search: checks that `kw` appears as an uppercase
/// sequence in `data` or that the lowercase form appears.
fn contains_keyword_ci(data: &[u8], kw: &[u8]) -> bool {
    if contains_subslice(data, kw) {
        return true;
    }
    let lower: Vec<u8> = kw.iter().map(|b| b.to_ascii_lowercase()).collect();
    contains_subslice(data, &lower)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_php() {
        let (kind, conf, ext) = classify(b"<?php\necho 'hello';");
        assert_eq!(kind, TextKind::Php);
        assert!(conf >= 99);
        assert_eq!(ext, "php");
    }

    #[test]
    fn classify_shebang_sh() {
        let (kind, conf, ext) = classify(b"#!/bin/sh\necho hello");
        assert_eq!(kind, TextKind::Script);
        assert!(conf >= 97);
        assert_eq!(ext, "sh");
    }

    #[test]
    fn classify_shebang_python() {
        let (kind, _, ext) = classify(b"#!/usr/bin/python3\nimport os");
        assert_eq!(kind, TextKind::Script);
        assert_eq!(ext, "py");
    }

    #[test]
    fn classify_shebang_ruby() {
        let (kind, _, ext) = classify(b"#!/usr/bin/ruby\nputs 'hello'");
        assert_eq!(kind, TextKind::Script);
        assert_eq!(ext, "rb");
    }

    #[test]
    fn classify_shebang_node() {
        let (kind, _, ext) = classify(b"#!/usr/bin/env node\nconsole.log('hi')");
        assert_eq!(kind, TextKind::Script);
        assert_eq!(ext, "js");
    }

    #[test]
    fn classify_xml() {
        let (kind, conf, _) = classify(b"<?xml version=\"1.0\"?><root/>");
        assert_eq!(kind, TextKind::Markup);
        assert!(conf >= 99);
    }

    #[test]
    fn classify_html() {
        let (kind, _, ext) = classify(b"<!DOCTYPE html>\n<html>");
        assert_eq!(kind, TextKind::Markup);
        assert_eq!(ext, "html");
    }

    #[test]
    fn classify_json() {
        let (kind, _, ext) = classify(b"{\"name\": \"foo\", \"value\": 42}");
        assert_eq!(kind, TextKind::Json);
        assert_eq!(ext, "json");
    }

    #[test]
    fn classify_yaml() {
        let (kind, conf, ext) = classify(b"---\ntitle: hello\nauthor: world\n");
        assert_eq!(kind, TextKind::Yaml);
        assert!(conf >= 90);
        assert_eq!(ext, "yaml");
    }

    #[test]
    fn classify_sql() {
        let data = b"SELECT * FROM users WHERE id = 1;\nINSERT INTO log VALUES (1);\nCREATE TABLE foo (id INT);";
        let (kind, _, ext) = classify(data);
        assert_eq!(kind, TextKind::Sql);
        assert_eq!(ext, "sql");
    }

    #[test]
    fn classify_c_source() {
        let data = b"#include <stdio.h>\n#define MAX 100\ntypedef struct { int x; } Point;\nvoid main() {}";
        let (kind, _, ext) = classify(data);
        assert_eq!(kind, TextKind::CSource);
        assert_eq!(ext, "c");
    }

    #[test]
    fn classify_markdown() {
        let data = b"# Title\n\nSome **bold** text and a [link](http://example.com)\n";
        let (kind, _, ext) = classify(data);
        assert_eq!(kind, TextKind::Markdown);
        assert_eq!(ext, "md");
    }

    #[test]
    fn classify_generic() {
        let (kind, _, ext) = classify(b"just some plain text with no special structure");
        assert_eq!(kind, TextKind::Generic);
        assert_eq!(ext, "txt");
    }
}
