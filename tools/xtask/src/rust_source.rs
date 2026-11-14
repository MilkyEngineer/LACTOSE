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

/// An out-of-line module declaration, `mod name;`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Module {
    /// The module's name.
    pub(crate) name: String,
    /// The file a `path` attribute on it names, as written.
    pub(crate) path: Option<String>,
    /// Whether it is compiled only for tests (see [`items`]).
    pub(crate) test_only: bool,
    /// The inline modules (`mod name { ... }`) it is declared in, outermost first.
    pub(crate) inline: Vec<InlineModule>,
}

/// An inline module, `mod name { ... }`, around a declaration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct InlineModule {
    /// The module's name.
    pub(crate) name: String,
    /// The directory a `path` attribute on it names, as written.
    pub(crate) path: Option<String>,
}

/// An `include!` of a string literal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Include {
    /// The file it names, as written.
    pub(crate) path: String,
    /// Whether it is compiled only for tests (see [`items`]).
    pub(crate) test_only: bool,
}

/// A trait implementation, `impl ... Trait for Type`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Impl {
    /// The line of its `impl`, from 1.
    pub(crate) line: usize,
    /// Whether it is compiled only for tests (see [`items`]).
    pub(crate) test_only: bool,
    /// The last segment of the trait's path, without generic arguments.
    pub(crate) trait_name: String,
    /// The last segment of the implementing type's path, without generic arguments; empty
    /// for a type that is not a path (a tuple, an array).
    pub(crate) self_type: String,
    /// The names of its type and const parameters.
    pub(crate) generics: Vec<String>,
    /// Each `type Name = Value;` in its body, as the name and the value without comments,
    /// with one space between two adjacent words and none elsewhere.
    pub(crate) types: Vec<(String, String)>,
}

/// What [`items`] finds in one file.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct Items {
    /// Each trait implementation, in order.
    pub(crate) impls: Vec<Impl>,
    /// Each out-of-line module declaration, in order.
    pub(crate) modules: Vec<Module>,
    /// Each `include!` of a string literal, in order.
    pub(crate) includes: Vec<Include>,
}

/// The trait implementations, out-of-line modules and literal `include!`s of `source`. An
/// item is compiled only for tests if a `cfg` on it, on an item around it or at the top of
/// the file holds only under `test` or a feature of `features` (`any` of such, or `all`
/// with one).
///
/// Token reading, not parsing: an item a macro expands to is not seen, a trait imported
/// under another name is read under that name, an `include!` of anything but a literal is
/// not followed, and an `impl Trait for` header inside a `macro_rules!` body is reported as
/// if it were an implementation.
pub(crate) fn items(source: &str, features: &[String]) -> Items {
    let bytes = source.as_bytes();
    let mut found = Items::default();
    // One entry per open brace; the first is the file.
    let mut levels = vec![Level::default()];
    // The item being read: its tokens, where its `impl` is, and its outer attributes.
    let mut header: Vec<Token> = Vec::new();
    let mut impl_at = None;
    let mut gated = false;
    let mut path = None;
    // Open parentheses and brackets in the item, inside which a `;` ends nothing.
    let mut nesting = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let enclosing = levels.last().is_some_and(|level| level.test_only);
        let byte = bytes[i];
        if byte.is_ascii_whitespace() {
            i += 1;
        } else if let Some(end) = skip_comment(source, i) {
            i = end;
        } else if let Some((inner, open)) = attribute_start(bytes, i)
            && let Some(end) = skip_brackets(source, open)
        {
            let body = &source[open + 1..end - 1];
            if inner {
                if let Some(level) = levels.last_mut() {
                    level.test_only |= cfg_test_only(body, features);
                }
            } else {
                gated |= cfg_test_only(body, features);
                path = path.or_else(|| path_value(body));
            }
            i = end;
        } else if matches!(byte, b'{' | b'}') || byte == b';' && nesting == 0 {
            let test_only = enclosing || gated;
            for literal in included(&header, source) {
                found.includes.push(Include { path: literal, test_only });
            }
            let mut opened = None;
            if let (Some(at), b'{' | b';') = (impl_at, byte)
                && let Some(implementation) = implementation(&header[at..], source, test_only)
            {
                opened = Some(found.impls.len());
                found.impls.push(implementation);
            }
            if byte == b';'
                && let Some(index) = levels.last().and_then(|level| level.implementation)
                && let Some(alias) = associated_type(&header, source)
            {
                found.impls[index].types.push(alias);
            }
            let module = match header[..] {
                [.., keyword, name] if keyword.is(source, "mod") && name.kind == Kind::Word => {
                    Some(name.text(source).to_owned())
                }
                _ => None,
            };
            match byte {
                b';' => {
                    if let Some(name) = module {
                        found.modules.push(Module {
                            name,
                            path: path.take(),
                            test_only,
                            inline: levels.iter().filter_map(|l| l.module.clone()).collect(),
                        });
                    }
                }
                b'{' => {
                    let module = module.map(|name| InlineModule { name, path: path.take() });
                    levels.push(Level { test_only, module, implementation: opened, nesting });
                    nesting = 0;
                }
                _ => {
                    if levels.len() > 1 {
                        nesting = levels.pop().map_or(0, |level| level.nesting);
                    }
                }
            }
            (header, impl_at, gated, path) = (Vec::new(), None, false, None);
            i += 1;
        } else if let Some(end) = skip_literal(source, i).or_else(|| {
            (byte == b'b' && bytes.get(i + 1) == Some(&b'r'))
                .then(|| skip_raw_string(bytes, i + 1))
                .flatten()
        }) {
            header.push(Token { start: i, end, kind: Kind::Literal });
            i = end;
        } else if is_ident(byte) {
            let end = bytes[i..].iter().position(|b| !is_ident(*b)).map_or(bytes.len(), |n| i + n);
            if &source[i..end] == "impl" && impl_at.is_none() {
                impl_at = Some(header.len());
            }
            header.push(Token { start: i, end, kind: Kind::Word });
            i = end;
        } else {
            let end = i + source[i..].chars().next().map_or(1, char::len_utf8);
            match byte {
                b'(' | b'[' => nesting += 1,
                b')' | b']' => nesting = nesting.saturating_sub(1),
                _ => {}
            }
            header.push(Token { start: i, end, kind: Kind::Punct });
            i = end;
        }
    }
    found
}

/// One open brace of [`items`].
#[derive(Default)]
struct Level {
    /// Whether what it holds is compiled only for tests.
    test_only: bool,
    /// The inline module it opens.
    module: Option<InlineModule>,
    /// The index in [`Items::impls`] of the implementation whose body it opens.
    implementation: Option<usize>,
    /// The parentheses and brackets open outside it.
    nesting: usize,
}

/// A word, a literal or one other character of [`items`]' source, by its byte range.
#[derive(Clone, Copy)]
struct Token {
    start: usize,
    end: usize,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Literal,
    Punct,
}

impl Token {
    fn text(self, source: &str) -> &str {
        &source[self.start..self.end]
    }

    fn is(self, source: &str, text: &str) -> bool {
        self.kind != Kind::Literal && self.text(source) == text
    }
}

/// The change in angle, parenthesis and bracket depth at `tokens[at]`. The `>` of `->`
/// closes nothing.
fn depth_change(tokens: &[Token], at: usize, source: &str) -> isize {
    let token = tokens[at];
    if token.kind != Kind::Punct {
        return 0;
    }
    match token.text(source) {
        "<" | "(" | "[" => 1,
        ">" if at > 0 && tokens[at - 1].is(source, "-") && tokens[at - 1].end == token.start => 0,
        ">" | ")" | "]" => -1,
        _ => 0,
    }
}

/// The trait implementation whose header is `tokens`, from its `impl` to the `{` or `;`
/// that ends it; `None` for an inherent implementation or a header that is not one.
fn implementation(tokens: &[Token], source: &str, test_only: bool) -> Option<Impl> {
    let mut generics = Vec::new();
    let mut at = 1;
    // `impl<...>`: each parameter's name follows the `<` or a `,` at depth one; a lifetime's
    // follows a `'` and is skipped.
    if tokens.get(at).is_some_and(|token| token.is(source, "<")) {
        let mut depth = 0isize;
        let mut expecting = false;
        while at < tokens.len() {
            depth += depth_change(tokens, at, source);
            let token = tokens[at];
            at += 1;
            if depth == 0 {
                break;
            }
            if depth == 1 && (token.is(source, "<") || token.is(source, ",")) {
                expecting = true;
            } else if expecting && token.kind == Kind::Word && !token.is(source, "const") {
                generics.push(token.text(source).to_owned());
                expecting = false;
            } else if token.kind != Kind::Word {
                expecting = false;
            }
        }
    }
    // The trait's path runs to the first `for` at depth zero that is not `for<...>`.
    let mut depth = 0isize;
    let mut trait_name = None;
    let mut split = None;
    for index in at..tokens.len() {
        let token = tokens[index];
        if depth == 0
            && token.is(source, "for")
            && !tokens.get(index + 1).is_some_and(|next| next.is(source, "<"))
        {
            split = Some(index);
            break;
        }
        if depth == 0 && token.kind == Kind::Word {
            trait_name = Some(token.text(source));
        }
        depth += depth_change(tokens, index, source);
    }
    let (split, trait_name) = (split?, trait_name?);
    let mut depth = 0isize;
    let mut self_type = "";
    for index in split + 1..tokens.len() {
        let token = tokens[index];
        if depth == 0 && token.kind == Kind::Word {
            if token.is(source, "where") {
                break;
            }
            self_type = token.text(source);
        }
        depth += depth_change(tokens, index, source);
    }
    Some(Impl {
        line: source[..tokens[0].start].matches('\n').count() + 1,
        test_only,
        trait_name: trait_name.to_owned(),
        self_type: self_type.to_owned(),
        generics,
        types: Vec::new(),
    })
}

/// The name and value of `type Name = Value;` if `tokens`, an item without its `;`, is one.
fn associated_type(tokens: &[Token], source: &str) -> Option<(String, String)> {
    match tokens {
        [keyword, name, equals, value @ ..]
            if keyword.is(source, "type")
                && name.kind == Kind::Word
                && equals.is(source, "=")
                && !value.is_empty() =>
        {
            // The tokens without whitespace or comments, but with a space between two words,
            // so `<L as Loader>::Asset` keeps `L` a word of its own.
            let mut text = String::new();
            for (at, token) in value.iter().enumerate() {
                if at > 0 && token.kind != Kind::Punct && value[at - 1].kind != Kind::Punct {
                    text.push(' ');
                }
                text.push_str(token.text(source));
            }
            Some((name.text(source).to_owned(), text))
        }
        _ => None,
    }
}

/// The literal of each `include!("...")` among `tokens`.
fn included(tokens: &[Token], source: &str) -> Vec<String> {
    tokens
        .windows(5)
        .filter(|w| {
            w[0].is(source, "include")
                && w[1].is(source, "!")
                && w[2].is(source, "(")
                && w[3].kind == Kind::Literal
                && w[4].is(source, ")")
        })
        .filter_map(|w| string_value(source, w[3].start))
        .collect()
}

/// Whether an attribute's text is a `cfg` that holds only under `test` or a feature of
/// `features`.
fn cfg_test_only(body: &str, features: &[String]) -> bool {
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    compact
        .strip_prefix("cfg(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|predicate| holds_only_for_tests(predicate, features))
}

fn holds_only_for_tests(predicate: &str, features: &[String]) -> bool {
    if predicate == "test" {
        return true;
    }
    if let Some(feature) = predicate.strip_prefix("feature=\"").and_then(|r| r.strip_suffix('"')) {
        return features.iter().any(|listed| listed == feature);
    }
    let operands = |op: &str| {
        let inner = predicate.strip_prefix(op)?.strip_prefix('(')?.strip_suffix(')')?;
        let mut parts = Vec::new();
        let (mut depth, mut start) = (0usize, 0);
        for (at, c) in inner.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&inner[start..at]);
                    start = at + 1;
                }
                _ => {}
            }
        }
        parts.push(&inner[start..]);
        parts.retain(|part| !part.is_empty());
        Some(parts)
    };
    if let Some(parts) = operands("any") {
        return !parts.is_empty() && parts.iter().all(|part| holds_only_for_tests(part, features));
    }
    if let Some(parts) = operands("all") {
        return parts.iter().any(|part| holds_only_for_tests(part, features));
    }
    false
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

    fn found(source: &str, features: &[&str]) -> Items {
        let features: Vec<String> = features.iter().map(|f| (*f).to_owned()).collect();
        items(source, &features)
    }

    /// The line of each implementation of `Asset` and whether it is compiled only for tests.
    fn impls(source: &str, features: &[&str]) -> Vec<(usize, bool)> {
        found(source, features)
            .impls
            .into_iter()
            .filter(|implementation| implementation.trait_name == "Asset")
            .map(|implementation| (implementation.line, implementation.test_only))
            .collect()
    }

    #[test]
    fn an_implementation_is_found_under_any_path_and_header() {
        let source = "impl Asset for A {}\n\
                      impl<T: Copy> crate::kinds::Asset for B<T> where T: Default {}\n\
                      unsafe impl ::k::Asset for C {}\n\
                      impl Asset for D;\n";
        assert_eq!(impls(source, &[]), [(1, false), (2, false), (3, false), (4, false)]);
        for source in [
            "impl Assets for A {}\n",
            "impl OtherAsset for A {}\n",
            "impl A { fn asset() -> Asset { for _ in 0..1 {} Asset } }\n",
            "fn f() { let asset = Asset; for x in xs {} }\n",
            "// impl Asset for A {}\n",
            "const S: &str = \"impl Asset for A {}\";\n",
            "const S: &[u8] = br\"impl Asset for A {}\";\n",
        ] {
            assert_eq!(impls(source, &[]), [], "{source:?}");
        }
    }

    #[test]
    fn an_implementation_compiled_only_for_tests_says_so() {
        let source = "#[cfg(test)]\nmod tests {\n    impl Asset for A {}\n}\n\
                      #[cfg(test)]\nimpl Asset for B {}\n\
                      #[cfg(any(test, feature = \"bench\"))]\nimpl Asset for C {}\n\
                      #[cfg(all(unix, test))]\nimpl Asset for D {}\n\
                      #[cfg(any(test, unix))]\nimpl Asset for E {}\n\
                      #[cfg(not(test))]\nimpl Asset for F {}\n\
                      mod m {\n    #![cfg(test)]\n    impl Asset for G {}\n}\n\
                      impl Asset for H {}\n";
        assert_eq!(
            impls(source, &["bench"]),
            [
                (3, true),
                (6, true),
                (8, true),
                (10, true),
                (12, false),
                (14, false),
                (17, true),
                (19, false),
            ]
        );
        assert_eq!(impls(source, &[])[2], (8, false), "`bench` is not a test feature here");
        assert_eq!(impls("#![cfg(test)]\nimpl Asset for A {}\n", &[]), [(2, true)]);
    }

    #[test]
    fn out_of_line_modules_are_found_with_their_path_and_gate() {
        let source = "mod a;\npub(crate) mod b {}\n#[path = \"x/c.rs\"]\nmod c;\n\
                      #[cfg(test)]\nmod d;\nmod e { mod f; }\n#[cfg(feature = \"dev\")] mod g;\n";
        // An inline module around it is written `name`, or `name=path` under a `#[path]`.
        let module = |name: &str, path: Option<&str>, test_only, inline: &[&str]| Module {
            name: name.to_owned(),
            path: path.map(str::to_owned),
            test_only,
            inline: inline
                .iter()
                .map(|m| {
                    let (name, path) = m.split_once('=').map_or((*m, None), |(n, p)| (n, Some(p)));
                    InlineModule { name: name.to_owned(), path: path.map(str::to_owned) }
                })
                .collect(),
        };
        assert_eq!(
            found(source, &["dev"]).modules,
            [
                module("a", None, false, &[]),
                module("c", Some("x/c.rs"), false, &[]),
                module("d", None, true, &[]),
                module("f", None, false, &["e"]),
                module("g", None, true, &[]),
            ]
        );
        // The inline modules around a declaration, but not the other blocks.
        let source = "#[cfg(test)]\nmod t {\n    mod u { fn f() {} mod v; }\n    impl X { }\n    mod w;\n}\n";
        assert_eq!(
            found(source, &[]).modules,
            [module("v", None, true, &["t", "u"]), module("w", None, true, &["t"])]
        );
        // Review round 2, R1: an inline module keeps its own `#[path]`, and only its own.
        let source = "#[path = \"../gen\"]\nmod g {\n    #[path = \"k.rs\"]\n    mod kinds;\n    mod h { mod i; }\n}\nmod j;\n";
        assert_eq!(
            found(source, &[]).modules,
            [
                module("kinds", Some("k.rs"), false, &["g=../gen"]),
                module("i", None, false, &["g=../gen", "h"]),
                module("j", None, false, &[]),
            ]
        );
    }

    #[test]
    fn an_implementation_knows_its_trait_type_parameters_and_associated_types() {
        let source = "impl<'a, T: Fn(&'a u8) -> Vec<u8>, const N: usize> crate::x::Loader<T> \
                      for k::Kind<'a, T, N> where T: for<'b> Fn(&'b u8) {\n\
                          type Asset = lactose_anim::Clip; // c\n\
                          type Error = Vec< T >;\n\
                          type Source = < T as /* c */ crate::X >::Y;\n\
                          fn f() { type Inner = u8; let _ = [0u8; 2]; }\n\
                      }\n\
                      impl Other for [u8; 4] { type Asset = A; }\n\
                      impl<T> Inherent<T> { type Asset = A; }\n\
                      impl !Send for (A, B) {}\n";
        let words = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect();
        let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        let implementation = |line, trait_name: &str, generics, types| Impl {
            line,
            test_only: false,
            trait_name: trait_name.to_owned(),
            self_type: String::new(),
            generics,
            types,
        };
        assert_eq!(
            found(source, &[]).impls,
            [
                Impl {
                    self_type: "Kind".to_owned(),
                    ..implementation(
                        1,
                        "Loader",
                        words(&["T", "N"]),
                        vec![
                            pair("Asset", "lactose_anim::Clip"),
                            pair("Error", "Vec<T>"),
                            pair("Source", "<T as crate::X>::Y"),
                        ],
                    )
                },
                implementation(7, "Other", Vec::new(), vec![pair("Asset", "A")]),
                implementation(9, "Send", Vec::new(), Vec::new()),
            ]
        );
    }

    #[test]
    fn a_literal_include_is_found_with_its_gate() {
        let source = "include!(\"a.rs\");\n#[cfg(test)]\nmod t { include!(r\"b.rs\"); }\n\
                      fn f() -> u8 { include!(\"c.rs\") }\n\
                      include!(concat!(env!(\"OUT_DIR\"), \"/d.rs\"));\n\
                      const S: &str = include_str!(\"e.rs\");\n";
        let include = |path: &str, test_only| Include { path: path.to_owned(), test_only };
        assert_eq!(
            found(source, &[]).includes,
            [include("a.rs", false), include("b.rs", true), include("c.rs", false)]
        );
    }

    #[test]
    fn a_semicolon_inside_brackets_ends_no_item() {
        // The gate on the constant is the constant's alone.
        let source = "#[cfg(test)]\nconst N: [u8; 2] = [0; 2];\nimpl Asset for A {}\n\
                      #[cfg(test)]\nfn f() -> [u8; 2] { [0; 2] }\nimpl Asset for B {}\n";
        assert_eq!(impls(source, &[]), [(3, false), (6, false)]);
    }
}
