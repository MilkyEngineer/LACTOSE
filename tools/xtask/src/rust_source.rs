//! Just enough reading of Rust source to find attributes: where they are, whether they are
//! inner or outer, and what they sit on. Comments, string literals and character literals are
//! skipped, so an attribute that is only quoted or commented out is not found.
//!
//! This is a scanner, not a parser. It does not expand macros, so an attribute written by a
//! macro is invisible to it.

/// One attribute of a source file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Attribute {
    /// True for `#![...]`, false for `#[...]`.
    pub(crate) inner: bool,
    /// The text between the brackets with whitespace and comments removed and the contents of
    /// string literals emptied, so `allow(dead_code /* c */, reason = "a, b")` reads
    /// `allow(dead_code,reason="")`.
    pub(crate) body: String,
    /// The line the attribute starts on, from 1.
    pub(crate) line: usize,
    /// True for an inner attribute that precedes every item and every outer attribute of the
    /// file, which is where an attribute applies to the whole crate or module file.
    pub(crate) leading: bool,
    /// True for an outer attribute whose item is a `mod`.
    pub(crate) on_module: bool,
    /// For an outer attribute on a `mod`, the file a `path = "..."` in it names, as written:
    /// `#[path = "..."]` itself, or the same inside a `cfg_attr`.
    pub(crate) path: Option<String>,
}

/// Every attribute of `source`, in order.
pub(crate) fn attributes(source: &str) -> Vec<Attribute> {
    let bytes = source.as_bytes();
    let mut attributes = Vec::new();
    let mut seen_item = false;
    // A shebang line is `#!` not followed by `[`.
    let mut i = match attribute_start(bytes, 0) {
        None if source.starts_with("#!") => source.find('\n').unwrap_or(bytes.len()),
        _ => 0,
    };
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
        } else if let Some(end) = skip_comment(source, i) {
            i = end;
        } else if let Some((inner, open)) = attribute_start(bytes, i)
            && let Some(end) = skip_brackets(source, open)
        {
            let text = &source[open + 1..end - 1];
            let on_module = !inner && next_item_is_module(source, end);
            attributes.push(Attribute {
                inner,
                body: code_only(text),
                line: source[..i].matches('\n').count() + 1,
                leading: inner && !seen_item,
                on_module,
                path: on_module.then(|| path_value(text)).flatten(),
            });
            seen_item |= !inner;
            i = end;
        } else {
            seen_item = true;
            let width = source[i..].chars().next().map_or(1, char::len_utf8);
            i = skip_literal(source, i).unwrap_or(i + width);
        }
    }
    attributes
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// If an attribute starts at `i`, whether it is inner and the index of its `[`.
fn attribute_start(bytes: &[u8], i: usize) -> Option<(bool, usize)> {
    if bytes.get(i) != Some(&b'#') {
        return None;
    }
    let mut j = skip_whitespace(bytes, i + 1);
    let inner = bytes.get(j) == Some(&b'!');
    if inner {
        j = skip_whitespace(bytes, j + 1);
    }
    (bytes.get(j) == Some(&b'[')).then_some((inner, j))
}

/// If a comment starts at `i`, the index after it. Block comments nest.
fn skip_comment(source: &str, i: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if source[i..].starts_with("//") {
        return Some(source[i..].find('\n').map_or(bytes.len(), |n| i + n));
    }
    if !source[i..].starts_with("/*") {
        return None;
    }
    let mut depth = 0usize;
    let mut j = i;
    while j < bytes.len() {
        if source[j..].starts_with("/*") {
            depth += 1;
            j += 2;
        } else if source[j..].starts_with("*/") {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return Some(j);
            }
        } else {
            j += 1;
            while !source.is_char_boundary(j) {
                j += 1;
            }
        }
    }
    Some(bytes.len())
}

/// If a string, raw string or character literal starts at `i`, the index after it. A `'`
/// that starts a lifetime is not a literal.
fn skip_literal(source: &str, i: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let before = |n: usize| i.checked_sub(n).map_or(0, |j| bytes[j]);
    match bytes[i] {
        b'"' => Some(skip_string(bytes, i)),
        // `r"…"`, and `br"…"` whose `b` was stepped over as an ordinary byte.
        b'r' if !is_ident(before(1)) || (before(1) == b'b' && !is_ident(before(2))) => {
            skip_raw_string(bytes, i)
        }
        b'\'' => {
            let rest = &source[i + 1..];
            if let Some(escaped) = rest.strip_prefix('\\') {
                // The escaped character may itself be a quote; the literal closes at the
                // next quote after it.
                let close = escaped.char_indices().skip(1).find(|&(_, c)| c == '\'');
                return Some(close.map_or(bytes.len(), |(n, _)| i + 2 + n + 1));
            }
            let c = rest.chars().next()?;
            rest[c.len_utf8()..].starts_with('\'').then_some(i + 1 + c.len_utf8() + 1)
        }
        _ => None,
    }
}

/// `i` is at an opening `"`. Returns the index after the closing one.
fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// `i` is at an `r` that may open a raw string (`r"…"`, `r#"…"#`). Returns the index after
/// the literal, or `None` if no raw string starts here.
fn skip_raw_string(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    let hashes = j - (i + 1);
    if bytes.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while j < bytes.len() {
        let closes = bytes[j] == b'"'
            && bytes[j + 1..].iter().take(hashes).filter(|b| **b == b'#').count() == hashes;
        if closes {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(bytes.len())
}

/// `i` is at `[` or `(`. Returns the index after the matching close, skipping string
/// literals and comments, or `None` if it never closes.
fn skip_brackets(source: &str, mut i: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let (open, close) = if bytes[i] == b'(' { (b'(', b')') } else { (b'[', b']') };
    let mut depth = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        // `/` is ASCII, so `i` is a character boundary for `skip_comment`.
        if bytes[i] == b'/'
            && let Some(end) = skip_comment(source, i)
        {
            i = end;
            continue;
        }
        if bytes[i] == open {
            depth += 1;
        } else if bytes[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// `text` without whitespace or comments and with the contents of its string literals removed.
fn code_only(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            out.push_str("\"\"");
            i = skip_string(bytes, i);
        } else if let Some(end) = skip_comment(text, i) {
            i = end;
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            if !c.is_whitespace() {
                out.push(c);
            }
            i += c.len_utf8();
        }
    }
    out
}

/// The string a `path = ...` key in `text`, an attribute's contents, names, wherever the key
/// stands. Of the escapes only `\\` and `\"` are undone; a path needs no other.
fn path_value(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let key = bytes[i..].starts_with(b"path")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && !bytes.get(i + 4).copied().is_some_and(is_ident);
        if key {
            let equals = skip_whitespace(bytes, i + 4);
            if bytes.get(equals) == Some(&b'=') {
                return string_value(text, skip_whitespace(bytes, equals + 1));
            }
        }
        // A literal starts only at an ASCII byte, so stepping by bytes slices no character.
        i = skip_literal(text, i).unwrap_or(i + 1);
    }
    None
}

/// The contents of the string or raw string literal that starts at `at`.
fn string_value(text: &str, at: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let (first, end, hashes) = match bytes.get(at)? {
        b'"' => (at + 1, skip_string(bytes, at), 0),
        b'r' => {
            let hashes = bytes[at + 1..].iter().take_while(|b| **b == b'#').count();
            (at + 2 + hashes, skip_raw_string(bytes, at)?, hashes)
        }
        _ => return None,
    };
    // The closing quote, which an unterminated literal lacks.
    let last = end.checked_sub(1 + hashes).filter(|&last| last >= first)?;
    if bytes.get(last) != Some(&b'"') {
        return None;
    }
    let raw = &text[first..last];
    if bytes[at] == b'r' {
        return Some(raw.to_owned());
    }
    let mut value = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (c, chars.clone().next()) {
            ('\\', Some(escaped @ ('\\' | '"'))) => {
                value.push(escaped);
                chars.next();
            }
            _ => value.push(c),
        }
    }
    Some(value)
}

/// Whether the item after the outer attribute that ends at `from` is a `mod`, looking past
/// further attributes, comments and a visibility.
fn next_item_is_module(source: &str, from: usize) -> bool {
    let bytes = source.as_bytes();
    let mut i = from;
    loop {
        i = skip_whitespace(bytes, i);
        if let Some(end) = skip_comment(source, i) {
            i = end;
        } else if let Some((_, open)) = attribute_start(bytes, i)
            && let Some(end) = skip_brackets(source, open)
        {
            i = end;
        } else {
            break;
        }
    }
    let keyword = |i: usize, word: &str| {
        source[i..].starts_with(word) && !bytes.get(i + word.len()).copied().is_some_and(is_ident)
    };
    if keyword(i, "pub") {
        i = skip_whitespace(bytes, i + 3);
        if bytes.get(i) == Some(&b'(') {
            i = skip_brackets(source, i).unwrap_or(bytes.len());
        }
        i = skip_whitespace(bytes, i);
    }
    i < bytes.len() && keyword(i, "mod")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bodies(source: &str) -> Vec<String> {
        attributes(source).into_iter().map(|a| a.body).collect()
    }

    #[test]
    fn attributes_in_comments_and_literals_are_not_found() {
        for source in [
            "// #![forbid(unsafe_code)]\n",
            "/* #![forbid(unsafe_code)] /* nested */ #[x] */\n",
            "const S: &str = \"#![forbid(unsafe_code)]\";\n",
            "const S: &str = r#\"#![forbid(\"unsafe_code\")]\"#;\n",
            "const S: &[u8] = br\"#[x]\";\n",
            "#!/usr/bin/env run-cargo-script\n",
        ] {
            assert_eq!(bodies(source), [] as [&str; 0], "{source:?}");
        }
    }

    #[test]
    fn bodies_lose_whitespace_and_string_contents() {
        let source = "//! Docs with a ' quote.\n#! [ allow( dead_code , reason = \"a ] b, warnings\" ) ]\nfn f<'a>(c: &'a str) -> char { '\\'' }\n#[cfg(test)] fn g() { let _ = '\"'; }\n#[x]\nfn h() {}\n#[allow(a /* ] \" */, // ]\n b)]\nfn i() {}\n";
        assert_eq!(
            bodies(source),
            ["allow(dead_code,reason=\"\")", "cfg(test)", "x", "allow(a,b)"]
        );
        assert_eq!(attributes(source)[0].line, 2);
        assert_eq!(attributes(source)[2].line, 5);
    }

    #[test]
    fn only_an_inner_attribute_before_every_item_is_leading() {
        let source = "//! Docs.\n#![a]\n/* c */ #![b]\n#[c]\nmod m {\n    #![d]\n}\n#![e]\n";
        let found: Vec<(String, bool, bool)> =
            attributes(source).into_iter().map(|a| (a.body, a.inner, a.leading)).collect();
        assert_eq!(
            found,
            [
                ("a".to_owned(), true, true),
                ("b".to_owned(), true, true),
                ("c".to_owned(), false, false),
                ("d".to_owned(), true, false),
                ("e".to_owned(), true, false),
            ]
        );
        // An item without attributes ends the leading run just the same.
        assert!(!attributes("use std::fmt;\n#![a]\n")[0].leading);
    }

    #[test]
    fn an_outer_attribute_knows_when_its_item_is_a_module() {
        let on_module = |source: &str| attributes(source)[0].on_module;
        assert!(on_module("#[a]\nmod m;"));
        assert!(on_module("#[a] pub mod m {}"));
        assert!(on_module("#[a]\n/// Docs.\n#[cfg(test)]\npub(crate) mod m;"));
        assert!(on_module("#[a] pub(in crate::x) mod m;"));
        assert!(!on_module("#[a] fn f() {}"));
        assert!(!on_module("#[a] pub fn module() {}"));
        assert!(!on_module("#[a] struct Model;"));
        assert!(!on_module("#[a]"));
        assert!(!attributes("#![a]\nmod m;")[0].on_module, "inner attributes sit on no item");
    }

    #[test]
    fn an_attribute_on_a_module_knows_the_file_it_names() {
        let path = |source: &str| attributes(source)[0].path.clone();
        let named = |source: &str, file: &str| assert_eq!(path(source).as_deref(), Some(file));
        named("#[path = \"../a/protocol.rs\"]\nmod protocol;", "../a/protocol.rs");
        named("#[cfg_attr(unix, path = \"u.rs\")] pub mod m;", "u.rs");
        named("#[path = r#\"raw \"x\".rs\"#] mod m;", "raw \"x\".rs");
        named("#[path = \"a\\\\b \\\".rs\"] mod m;", "a\\b \".rs");
        named("#[path=\"é/ü.rs\"] mod m;", "é/ü.rs");
        for source in [
            "#[doc = \"path = \\\"no.rs\\\"\"] mod m;",
            "#[module_path = \"no.rs\"] mod m;",
            "#[path = \"no.rs\"] fn f() {}",
            "#![path = \"no.rs\"] mod m;",
            "#[path] mod m;",
            "#[path = \"unterminated] mod m;",
        ] {
            assert_eq!(attributes(source).first().and_then(|a| a.path.clone()), None, "{source:?}");
        }
    }
}
