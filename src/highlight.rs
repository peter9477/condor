//! Syntax highlighting for source files and fenced code blocks in Markdown.
//!
//! Done here rather than in the page so documents stay script-free. Uses syntect's
//! built-in Sublime syntaxes and a pure-Rust regex engine. The output is HTML spans with
//! CSS classes; colours come from a light and a dark theme, chosen by the page's colour
//! scheme.

use std::sync::OnceLock;
use syntect::highlighting::ThemeSet;
use syntect::html::{css_for_theme_with_class_style, ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// Larger inputs are shown uncoloured: highlighting costs roughly 1 s per few MB.
const MAX_BYTES: usize = 2 << 20;
const STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

fn syntaxes() -> &'static SyntaxSet {
    static S: OnceLock<SyntaxSet> = OnceLock::new();
    S.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// A `<style>` element with the highlight colours (light, plus dark when preferred).
pub fn style_tag() -> &'static str {
    static C: OnceLock<String> = OnceLock::new();
    C.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        let css = |name: &str| {
            ts.themes.get(name).and_then(|t| css_for_theme_with_class_style(t, STYLE).ok()).unwrap_or_default()
        };
        format!(
            "<style>{}\n@media (prefers-color-scheme: dark) {{\n{}\n}}</style>",
            css("InspiredGitHub"),
            css("base16-ocean.dark")
        )
    })
}

fn usable(s: &'static SyntaxReference) -> Option<&'static SyntaxReference> {
    (s.name != "Plain Text").then_some(s)
}

/// Syntax for a file, by extension, or by its first line (`#!/usr/bin/env python`).
pub fn for_file(ext: &str, text: &str) -> Option<&'static SyntaxReference> {
    let ss = syntaxes();
    let ext = match ext {
        "svg" | "xsd" | "xsl" | "xslt" | "plist" => "xml",
        "xhtml" => "html",
        e => e,
    };
    let by_ext = if ext.is_empty() { None } else { ss.find_syntax_by_extension(ext) };
    let by_line = || text.lines().next().and_then(|l| ss.find_syntax_by_first_line(l));
    by_ext.or_else(by_line).and_then(usable)
}

/// Syntax for a Markdown code fence's language tag (`rust`, `py`, `c++`, ...).
pub fn for_lang(lang: &str) -> Option<&'static SyntaxReference> {
    let lang = lang.split([',', ' ']).next().unwrap_or("").trim();
    if lang.is_empty() {
        return None;
    }
    let ss = syntaxes();
    let alias = match lang.to_ascii_lowercase().as_str() {
        "c++" | "cxx" => "cpp".to_string(),
        "shell" | "console" => "sh".to_string(),
        l => l.to_string(),
    };
    ss.find_syntax_by_token(&alias).and_then(usable)
}

/// HTML for `code` (escaped, with highlight spans), or None if it's too big or fails.
pub fn to_html(code: &str, syntax: &SyntaxReference) -> Option<String> {
    if code.len() > MAX_BYTES {
        return None;
    }
    let mut g = ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes(), STYLE);
    for line in LinesWithEndings::from(code) {
        g.parse_html_for_line_which_includes_newline(line).ok()?;
    }
    Some(g.finalize())
}
