//! The nightly Miri job's split of a package (architecture section 20): the entries of the
//! `miri` job's matrix in `.github/workflows/nightly.yml` that name a package of the policy
//! file's `[miri_split]` must together run its library, each of its integration test targets
//! and its doc tests exactly once. Without the check only a comment keeps a split complete,
//! and a new test target that no entry's patterns select drops out of Miri unseen.
//!
//! The workflow is read as text, by a reader of the YAML the job is written in: block
//! mappings and sequences, plain and quoted scalars on one line, plain scalars over several,
//! and block scalars. It refuses what it does not read instead of guessing. An entry's `args`
//! are split as the run step's shell splits them, and its `--test` patterns are matched as
//! cargo matches them: a name with none of `*?[]` exactly, any other as a pattern of the
//! `glob` crate (see [`Pattern`]).

use std::collections::BTreeMap;

use crate::policy::Policy;
use crate::repo::Files;
use crate::workspace::{Package, Workspace};
use crate::{Finding, POLICY_FILE};

/// The repository-relative path of the nightly workflow.
pub const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// The keys from the workflow's root to the Miri job's list of matrix entries.
const ENTRIES: [&str; 5] = ["jobs", "miri", "strategy", "matrix", "include"];

/// Checks the Miri entries of every package in `[miri_split]` against its targets.
#[must_use]
pub fn check(policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    if policy.miri_split.is_empty() {
        return findings;
    }
    let Some(text) = files.text(NIGHTLY) else {
        findings.push(Finding::new(
            NIGHTLY,
            format!("is missing; {POLICY_FILE} lists packages its Miri job splits"),
        ));
        return findings;
    };
    let entries = match entries(&text) {
        Ok(entries) => entries,
        Err(message) => {
            findings.push(Finding::new(NIGHTLY, message));
            return findings;
        }
    };
    for name in &policy.miri_split {
        match ws.package(name) {
            Some(package) => split(package, &entries, &mut findings),
            None => findings.push(Finding::new(
                POLICY_FILE,
                format!("[miri_split] lists `{name}`, which is no workspace member"),
            )),
        }
    }
    findings
}

/// One entry of the job's matrix.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Entry {
    /// The line of its `-`.
    line: usize,
    /// Its `name`, or empty.
    name: String,
    /// Its `args`, and the line of the key.
    args: Option<(usize, String)>,
}

impl Entry {
    fn label(&self) -> String {
        format!("`{}` (line {})", self.name, self.line)
    }
}

/// What a cargo invocation runs of a package's tests.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Run<'a> {
    Lib,
    Test(&'a str),
    Doc,
}

impl Run<'_> {
    fn describe(self) -> String {
        match self {
            Run::Lib => "the library's unit tests".to_owned(),
            Run::Test(name) => format!("test target `{name}`"),
            Run::Doc => "the doc tests".to_owned(),
        }
    }
}

/// The finding for each of `package`'s runs that the entries naming it do not run exactly
/// once, and for each entry naming it that the check cannot read.
fn split(package: &Package, entries: &[Entry], findings: &mut Vec<Finding>) {
    let mut runs: BTreeMap<Run<'_>, Vec<&Entry>> = BTreeMap::new();
    if package.lib {
        runs.insert(Run::Lib, Vec::new());
    }
    if package.doctests {
        runs.insert(Run::Doc, Vec::new());
    }
    for test in &package.tests {
        runs.insert(Run::Test(test), Vec::new());
    }
    let mut named = false;
    for entry in entries {
        let Some((line, args)) = &entry.args else { continue };
        let report = |message: String| {
            Finding::new(NIGHTLY, format!("line {line}: the args of `{}`: {message}", entry.name))
        };
        let words = match words(args) {
            Ok(words) => words,
            // Unread words could name the package, so the entry is reported whatever it names.
            Err(message) => {
                findings.push(report(message));
                continue;
            }
        };
        if package_of(&words) != Some(package.name.as_str()) {
            continue;
        }
        named = true;
        match selected(package, &words) {
            Ok(selected) => {
                for run in selected {
                    runs.entry(run).or_default().push(entry);
                }
            }
            Err(message) => findings.push(report(message)),
        }
    }
    if !named {
        findings.push(Finding::new(
            NIGHTLY,
            format!(
                "no entry of the `miri` job runs `{}`, which {POLICY_FILE} lists under \
                 [miri_split]",
                package.name
            ),
        ));
        return;
    }
    for (run, by) in runs {
        let message = match by.as_slice() {
            [_] => continue,
            [] => format!(
                "the `miri` entries for `{}` do not run {}; select it in one of them",
                package.name,
                run.describe()
            ),
            _ => format!(
                "the `miri` entries for `{}` run {} {} times: {}",
                package.name,
                run.describe(),
                by.len(),
                by.iter().map(|entry| entry.label()).collect::<Vec<_>>().join(", ")
            ),
        };
        findings.push(Finding::new(NIGHTLY, message));
    }
}

/// The package an entry's words name with `-p` or `--package`, if they name one.
fn package_of(words: &[String]) -> Option<&str> {
    let mut words = words.iter().map(String::as_str);
    while let Some(word) = words.next() {
        if word == "-p" || word == "--package" {
            return words.next();
        }
        if let Some(name) = word.strip_prefix("--package=").or_else(|| word.strip_prefix("-p")) {
            return Some(name);
        }
    }
    None
}

/// What `cargo miri test` with `words` runs of `package`, which they name: one item per
/// selection, so a target two patterns select is in it twice.
///
/// # Errors
/// A word the check does not read, a target cargo would refuse, or `--doc` beside another
/// target selection, which cargo refuses.
fn selected<'a>(package: &'a Package, words: &[String]) -> Result<Vec<Run<'a>>, String> {
    fn value<'w>(flag: &str, words: &mut impl Iterator<Item = &'w str>) -> Result<&'w str, String> {
        words.next().ok_or_else(|| format!("`{flag}` has no value"))
    }
    let (mut lib, mut doc, mut patterns, mut packages) = (false, false, Vec::new(), 0);
    let mut words = words.iter().map(String::as_str);
    while let Some(word) = words.next() {
        match word {
            "-p" | "--package" => {
                value(word, &mut words)?;
                packages += 1;
            }
            _ if word.starts_with("--package=") || (word.starts_with("-p") && word.len() > 2) => {
                packages += 1;
            }
            "--lib" => lib = true,
            "--doc" => doc = true,
            "--test" => patterns.push(value(word, &mut words)?),
            _ if word.starts_with("--test=") => patterns.push(&word["--test=".len()..]),
            _ => return Err(format!("the check does not know what `{word}` runs")),
        }
    }
    if packages != 1 {
        return Err("names more than one package, which the check does not read".to_owned());
    }
    if doc && (lib || !patterns.is_empty()) {
        return Err("mixes `--doc` with another target selection, which cargo refuses".to_owned());
    }
    if doc {
        if !package.doctests {
            return Err(format!("names `--doc`, but `{}` has no doc tests", package.name));
        }
        return Ok(vec![Run::Doc]);
    }
    if !lib && patterns.is_empty() {
        // Naming no target runs them all, the doc tests included.
        let tests = package.tests.iter().map(|test| Run::Test(test));
        let lib = package.lib.then_some(Run::Lib);
        return Ok(lib
            .into_iter()
            .chain(tests)
            .chain(package.doctests.then_some(Run::Doc))
            .collect());
    }
    let mut runs = Vec::new();
    if lib {
        if !package.lib {
            return Err(format!("names `--lib`, but `{}` has no library", package.name));
        }
        runs.push(Run::Lib);
    }
    for text in patterns {
        let before = runs.len();
        if Pattern::is_pattern(text) {
            let pattern = Pattern::new(text)
                .map_err(|e| format!("`--test {text}` is not a pattern cargo accepts: {e}"))?;
            runs.extend(package.tests.iter().filter(|t| pattern.matches(t)).map(|t| Run::Test(t)));
        } else if let Some(test) = package.tests.iter().find(|t| *t == text) {
            runs.push(Run::Test(test));
        }
        if runs.len() == before {
            return Err(format!("`--test {text}` selects no test target, which cargo refuses"));
        }
    }
    Ok(runs)
}

/// The words the run step's shell makes of `args`, which the workflow substitutes into the
/// command line as text: split at spaces and tabs, quotes removed.
///
/// # Errors
/// A line break, which ends the command, an unquoted glob character, which the shell would
/// expand against the checkout's files, and any other shell syntax the check does not read:
/// expansions, escapes, operators, redirections and comments.
fn words(args: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => words.extend(word.take()),
            '\'' | '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some(end) if end == c => break,
                        Some('$' | '`' | '\\') if c == '"' => {
                            return Err("a `$`, backtick or backslash inside double quotes, \
                                        which the check does not read"
                                .to_owned());
                        }
                        Some('\n') => return Err("a line break, which ends the command".to_owned()),
                        Some(inner) => word.push(inner),
                        None => return Err(format!("an unclosed {c}")),
                    }
                }
            }
            '*' | '?' | '[' => {
                return Err(format!(
                    "an unquoted `{c}`, which the shell would expand against the checkout's \
                     files; quote the word"
                ));
            }
            '\n' | '\r' => return Err("a line break, which ends the command".to_owned()),
            '$' | '`' | '\\' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{' | '}' | '#' | '~'
            | '!' => {
                return Err(format!("an unquoted `{c}`, shell syntax the check does not read"));
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// A `--test` pattern, matched as cargo 1.99 matches it: with `glob::Pattern` and its default
/// match options, so case-sensitively against the whole target name. `?` is one character,
/// `*` any run of them, `**` is `*` but only as the whole pattern, and `[...]` and `[!...]`
/// are a class and its complement: a `]` right after the opening is a member, and `a-z` is a
/// range unless the `-` is first or last. No character escapes another.
///
/// The cases of the unit tests below were each run through `cargo test --no-run --test` on
/// cargo 1.99.0, and the selections and refusals are cargo's. A pattern holding a path
/// separator is refused here, since `glob` treats `/` (and on Windows `\`) as one.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Pattern(Vec<Token>);

#[derive(Clone, PartialEq, Eq, Debug)]
enum Token {
    Char(char),
    AnyChar,
    AnySequence,
    /// The class's single characters and inclusive ranges, and whether it is complemented.
    Class {
        negated: bool,
        members: Vec<(char, char)>,
    },
}

impl Pattern {
    /// Whether cargo reads `text` as a pattern rather than as a target's name.
    fn is_pattern(text: &str) -> bool {
        text.contains(['*', '?', '[', ']'])
    }

    /// Parses `text` as `glob::Pattern::new` does.
    ///
    /// # Errors
    /// What `glob` refuses, with its message: `***`, a `**` beside another character, and a
    /// `[` with no class after it; and a path separator, which the check does not read.
    fn new(text: &str) -> Result<Self, String> {
        let chars: Vec<char> = text.chars().collect();
        if chars.iter().any(|&c| c == '/' || c == '\\') {
            return Err("a path separator, which the check does not read".to_owned());
        }
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '?' => {
                    tokens.push(Token::AnyChar);
                    i += 1;
                }
                '*' => {
                    let start = i;
                    while chars.get(i) == Some(&'*') {
                        i += 1;
                    }
                    match i - start {
                        1 => {}
                        2 if start == 0 && i == chars.len() => {}
                        2 => {
                            return Err(
                                "recursive wildcards must form a single path component".to_owned()
                            );
                        }
                        _ => {
                            return Err(
                                "wildcards are either regular `*` or recursive `**`".to_owned()
                            );
                        }
                    }
                    tokens.push(Token::AnySequence);
                }
                '[' => {
                    let negated = chars.get(i + 1) == Some(&'!');
                    // The first member follows `[` or `[!`, and may itself be `]`.
                    let first = i + 1 + usize::from(negated);
                    let close = chars
                        .get(first + 1..)
                        .and_then(|rest| rest.iter().position(|&c| c == ']'))
                        .map(|at| first + 1 + at);
                    let Some(close) = close.filter(|_| first < chars.len()) else {
                        return Err("invalid range pattern".to_owned());
                    };
                    tokens.push(Token::Class { negated, members: members(&chars[first..close]) });
                    i = close + 1;
                }
                c => {
                    tokens.push(Token::Char(c));
                    i += 1;
                }
            }
        }
        Ok(Self(tokens))
    }

    fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.chars().collect();
        matches_from(&self.0, &name)
    }
}

/// A class's members as `glob` reads them: `a-z` is a range wherever a character has a `-`
/// and another character after it, and every other character is itself.
fn members(chars: &[char]) -> Vec<(char, char)> {
    let mut members = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            members.push((chars[i], chars[i + 2]));
            i += 3;
        } else {
            members.push((chars[i], chars[i]));
            i += 1;
        }
    }
    members
}

fn matches_from(tokens: &[Token], name: &[char]) -> bool {
    let Some((token, rest)) = tokens.split_first() else {
        return name.is_empty();
    };
    match token {
        Token::AnySequence => (0..=name.len()).any(|skip| matches_from(rest, &name[skip..])),
        _ => {
            let Some((&c, tail)) = name.split_first() else {
                return false;
            };
            let one = match token {
                Token::Char(expected) => c == *expected,
                Token::AnyChar => true,
                Token::Class { negated, members } => {
                    members.iter().any(|&(low, high)| low <= c && c <= high) != *negated
                }
                Token::AnySequence => unreachable!("matched above"),
            };
            one && matches_from(rest, tail)
        }
    }
}

/// One line of the workflow: its number, its indentation and its text without either the
/// indentation or trailing white space (a carriage return included).
#[derive(Clone, Copy, Debug)]
struct Line<'a> {
    number: usize,
    indent: usize,
    text: &'a str,
}

impl Line<'_> {
    /// Whether the line holds structure: it is neither blank nor a comment.
    fn significant(&self) -> bool {
        !self.text.is_empty() && !self.text.starts_with('#')
    }
}

/// The entries of the Miri job's matrix.
///
/// # Errors
/// When the workflow has no such list, or the list holds YAML the reader does not read; the
/// message names the line.
fn entries(text: &str) -> Result<Vec<Entry>, String> {
    let lines: Vec<Line<'_>> = text
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            let text = raw.trim_start_matches(' ');
            Line { number: index + 1, indent: raw.len() - text.len(), text: text.trim_end() }
        })
        .collect();
    let mut body = lines.as_slice();
    for key in ENTRIES {
        body = child(body, key)
            .ok_or_else(|| format!("has no `{}` list for the check to read", ENTRIES.join(".")))?;
    }
    let mut entries = Vec::new();
    let Some(first) = body.iter().find(|line| line.significant()) else {
        return Ok(entries);
    };
    let at = first.indent;
    let mut rest = body;
    while let Some(start) = rest.iter().position(Line::significant) {
        let line = rest[start];
        let item =
            line.text.strip_prefix('-').filter(|item| item.is_empty() || item.starts_with(' '));
        let (Some(item), true) = (item, line.indent == at) else {
            return Err(format!(
                "line {}: an item of the matrix the check does not read",
                line.number
            ));
        };
        let end = block_end(&rest[start + 1..], at).map_or(rest.len(), |end| start + 1 + end);
        let mut first = line;
        first.text = item.trim_start_matches(' ');
        first.indent = line.indent + line.text.len() - first.text.len();
        entries.push(entry(line.number, first, &rest[start + 1..end])?);
        rest = &rest[end..];
    }
    Ok(entries)
}

/// The index in `lines` of the first significant line indented `parent` or less: where the
/// block of the line at `parent` ends.
fn block_end(lines: &[Line<'_>], parent: usize) -> Option<usize> {
    lines.iter().position(|line| line.significant() && line.indent <= parent)
}

/// The index in `lines` of the first line, a comment included, that is not blank and is
/// indented `key` or less: where the value of the key at `key` ends.
fn value_end(lines: &[Line<'_>], key: usize) -> Option<usize> {
    lines.iter().position(|line| !line.text.is_empty() && line.indent <= key)
}

/// The lines of the value of `key` in the mapping `body` holds, whose keys are at the indent
/// of its first significant line. Only a key with nothing after its colon has a block.
fn child<'a, 'b>(body: &'b [Line<'a>], key: &str) -> Option<&'b [Line<'a>]> {
    let indent = body.iter().find(|line| line.significant())?.indent;
    let start = body.iter().position(|line| {
        line.significant()
            && line.indent == indent
            && line.text.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')).is_some_and(
                |rest| {
                    let rest = rest.trim_start();
                    rest.is_empty() || rest.starts_with('#')
                },
            )
    })?;
    let rest = &body[start + 1..];
    Some(&rest[..block_end(rest, indent).unwrap_or(rest.len())])
}

/// One matrix entry: `first` is its first key's line, the text after the `-`; `rest` holds
/// its other lines.
fn entry(number: usize, first: Line<'_>, rest: &[Line<'_>]) -> Result<Entry, String> {
    let mut entry = Entry { line: number, name: String::new(), args: None };
    let lines: Vec<Line<'_>> = std::iter::once(first).chain(rest.iter().copied()).collect();
    let mut seen = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        at += 1;
        if !line.significant() {
            continue;
        }
        let unread =
            || format!("line {}: a line of the entry the check does not read", line.number);
        if line.indent != first.indent {
            return Err(unread());
        }
        let (key, value) = line.text.split_once(':').ok_or_else(unread)?;
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || !(value.is_empty() || value.starts_with(' '))
        {
            return Err(unread());
        }
        if seen.contains(&key) {
            return Err(format!("line {}: a second `{key}` in one entry", line.number));
        }
        seen.push(key);
        let end = value_end(&lines[at..], first.indent).map_or(lines.len(), |end| at + end);
        let more = &lines[at..end];
        at = end;
        match key {
            "name" => entry.name = scalar(value.trim_start(), more, line.number)?,
            "args" => {
                entry.args = Some((line.number, scalar(value.trim_start(), more, line.number)?))
            }
            _ => {}
        }
    }
    Ok(entry)
}

/// The value of a scalar whose text after the key's colon is `head` and whose further lines,
/// all indented past the key, are `more`.
///
/// # Errors
/// A flow collection, an anchor, alias or tag, a quoted scalar that does not end on its
/// line or holds a backslash, and a block scalar with an indentation indicator or a line
/// indented past its first.
fn scalar(head: &str, more: &[Line<'_>], number: usize) -> Result<String, String> {
    let unread = |what: &str| format!("line {number}: {what}, which the check does not read");
    let continued = more.iter().any(|line| !line.text.is_empty());
    match head.chars().next() {
        Some(style @ ('>' | '|')) => {
            let header = strip_comment(&head[1..]);
            let chomp = match header {
                "" => None,
                "-" => Some(false),
                "+" => Some(true),
                _ => return Err(unread("a block scalar header other than `>`, `|`, `-` and `+`")),
            };
            let indent = more.iter().find(|line| !line.text.is_empty()).map(|line| line.indent);
            if more.iter().any(|line| !line.text.is_empty() && Some(line.indent) != indent) {
                return Err(unread("a block scalar line indented past its first"));
            }
            // Literal keeps each line break; folded makes one between two lines a space, and
            // keeps each blank line as a line break.
            let mut value = String::new();
            for (at, line) in more.iter().enumerate() {
                let previous = at.checked_sub(1).map(|before| more[before].text);
                match (style, previous) {
                    ('|', Some(_)) => value.push('\n'),
                    ('|', None) => {}
                    _ if line.text.is_empty() => value.push('\n'),
                    (_, Some(before)) if !before.is_empty() => value.push(' '),
                    _ => {}
                }
                value.push_str(line.text);
            }
            let body = value.trim_end_matches('\n').len();
            match chomp {
                Some(false) => value.truncate(body),
                Some(true) => value.push('\n'),
                None => {
                    value.truncate(body);
                    if body > 0 {
                        value.push('\n');
                    }
                }
            }
            Ok(value)
        }
        Some(quote @ ('\'' | '"')) => {
            let mut value = String::new();
            let mut chars = head[1..].char_indices();
            loop {
                match chars.next() {
                    Some((at, c)) if c == quote => {
                        if quote == '\'' && head[1 + at + 1..].starts_with('\'') {
                            chars.next();
                            value.push('\'');
                            continue;
                        }
                        let after = &head[1 + at + 1..];
                        if !strip_comment(after).is_empty() || continued {
                            return Err(unread("text after a quoted scalar"));
                        }
                        return Ok(value);
                    }
                    Some((_, '\\')) if quote == '"' => {
                        return Err(unread("a backslash in a double-quoted scalar"));
                    }
                    Some((_, c)) => value.push(c),
                    None => return Err(unread("a quoted scalar that does not end on its line")),
                }
            }
        }
        Some('[' | '{' | '&' | '*' | '!' | '%' | '@' | '`') => {
            Err(unread("a flow collection, anchor, alias, tag or reserved indicator"))
        }
        _ => {
            // A plain scalar: its lines folded with single spaces, a blank line kept as a line
            // break. A comment may follow its first line only, and a `: ` in it, which makes
            // a mapping instead, is not read.
            let mut value = strip_comment(head).to_owned();
            let mut blank = false;
            for line in more {
                if line.text.is_empty() {
                    blank = true;
                    continue;
                }
                if line.text.starts_with('#') || strip_comment(line.text) != line.text {
                    return Err(unread("a comment inside a plain scalar"));
                }
                if !value.is_empty() {
                    value.push(if blank { '\n' } else { ' ' });
                }
                blank = false;
                value.push_str(line.text);
            }
            if value.contains(": ") || value.contains(":\n") || value.ends_with(':') {
                return Err(unread("a `: ` inside a plain scalar"));
            }
            Ok(value)
        }
    }
}

/// `text` without a trailing comment, which starts at a `#` after white space, and trimmed.
fn strip_comment(text: &str) -> &str {
    let text = text.trim();
    if text.starts_with('#') {
        return "";
    }
    text.find(" #").or_else(|| text.find("\t#")).map_or(text, |at| text[..at].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::fixture::{package, workspace};

    const NAMES: [&str; 13] = [
        "-a",
        "Cap",
        "a",
        "a1",
        "ab",
        "b",
        "e1",
        "erased",
        "erased_rows",
        "o",
        "ownership",
        "x-y",
        "z",
    ];

    fn selection(text: &str) -> Result<Vec<&'static str>, String> {
        let pattern = Pattern::new(text)?;
        Ok(NAMES.iter().copied().filter(|name| pattern.matches(name)).collect())
    }

    #[test]
    fn patterns_select_what_cargo_selects() {
        // Each case's targets are cargo 1.99.0's for `cargo test --no-run --test <pattern>` in
        // a package whose integration tests are `NAMES` (target names are case-sensitive).
        let all = &NAMES[..];
        let cases: [(&str, &[&str]); 25] = [
            ("[!eo]*", &["-a", "Cap", "a", "a1", "ab", "b", "x-y", "z"]),
            ("e*", &["e1", "erased", "erased_rows"]),
            ("?", &["a", "b", "o", "z"]),
            ("??", &["-a", "a1", "ab", "e1"]),
            ("*", all),
            ("**", all),
            ("?*", all),
            ("*?", all),
            ("*a*", &["-a", "Cap", "a", "a1", "ab", "erased", "erased_rows"]),
            ("[]a]*", &["a", "a1", "ab"]),
            (
                "[!]a]*",
                &["-a", "Cap", "b", "e1", "erased", "erased_rows", "o", "ownership", "x-y", "z"],
            ),
            ("[a-c]", &["a", "b"]),
            ("[c-a]", &[]),
            ("[a-]", &["a"]),
            ("[-a]*", &["-a", "a", "a1", "ab"]),
            ("[-]*", &["-a"]),
            (
                "[!-]*",
                &[
                    "Cap",
                    "a",
                    "a1",
                    "ab",
                    "b",
                    "e1",
                    "erased",
                    "erased_rows",
                    "o",
                    "ownership",
                    "x-y",
                    "z",
                ],
            ),
            ("[!a]", &["b", "o", "z"]),
            ("[!a-d]", &["o", "z"]),
            (
                "[!a-c]*",
                &["-a", "Cap", "e1", "erased", "erased_rows", "o", "ownership", "x-y", "z"],
            ),
            ("[a-ce]", &["a", "b"]),
            // `]-a` is the range from `]` to `a`, which holds no `-`.
            ("[]-a]*", &["a", "a1", "ab"]),
            ("[x]-y", &["x-y"]),
            ("[C]ap", &["Cap"]),
            ("e*s", &["erased_rows"]),
        ];
        for (pattern, expected) in cases {
            assert!(Pattern::is_pattern(pattern), "{pattern}");
            assert_eq!(selection(pattern).as_deref(), Ok(expected), "{pattern}");
        }
        assert_eq!(selection("*_*").as_deref(), Ok(&["erased_rows"][..]));
        assert_eq!(selection("a?").as_deref(), Ok(&["a1", "ab"][..]));
        assert_eq!(selection("[0-9a-b]?").as_deref(), Ok(&["a1", "ab"][..]));
    }

    #[test]
    fn patterns_cargo_refuses_are_refused() {
        // cargo 1.99.0: "cannot build glob pattern from `<pattern>`", with these messages.
        for (pattern, message) in [
            ("a**", "recursive wildcards must form a single path component"),
            ("**a", "recursive wildcards must form a single path component"),
            ("***", "wildcards are either regular `*` or recursive `**`"),
            ("[a", "invalid range pattern"),
            ("[!", "invalid range pattern"),
            ("[]", "invalid range pattern"),
            ("[!]", "invalid range pattern"),
            ("a[", "invalid range pattern"),
        ] {
            assert_eq!(Pattern::new(pattern), Err(message.to_owned()), "{pattern}");
        }
        assert!(Pattern::new("a/*").unwrap_err().contains("path separator"));
        assert!(Pattern::new("a\\*").unwrap_err().contains("path separator"));
        // Names cargo looks up exactly: none of `*?[]`.
        for name in ["x-y", "Cap", "", "\\a", "erased_rows"] {
            assert!(!Pattern::is_pattern(name), "{name}");
        }
        assert!(Pattern::is_pattern("q]r"));
    }

    fn ecs(tests: &[&str]) -> Package {
        let mut ecs = package("lactose-ecs", &[]);
        ecs.tests = tests.iter().map(|t| (*t).to_owned()).collect();
        ecs
    }

    fn policy() -> Policy {
        Policy { miri_split: vec!["lactose-ecs".to_owned()], ..Policy::default() }
    }

    fn run(package: Package, nightly: &str) -> Vec<String> {
        let mut files = Files::default();
        files.insert(NIGHTLY, nightly);
        check(&policy(), &workspace(vec![package]), &files)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// A nightly workflow whose Miri job has `entries`, each a list of `key: value` lines.
    fn nightly(entries: &[&[&str]]) -> String {
        let mut text = String::from(
            "name: nightly\n\njobs:\n  # The job.\n  miri:\n    name: miri\n    strategy:\n      \
             fail-fast: false\n      matrix:\n        include:\n",
        );
        for entry in entries {
            for (at, line) in entry.iter().enumerate() {
                let lead = if at == 0 { "          - " } else { "            " };
                text.push_str(&format!("{lead}{line}\n"));
            }
        }
        text.push_str(
            "    steps:\n      - run: cargo miri test ${{ matrix.args }}\n  loom:\n    name: x\n",
        );
        text
    }

    const TESTS: [&str; 6] =
        ["alloc", "erased", "erased_rows", "events", "obligation", "ownership"];

    fn main_split() -> String {
        nightly(&[
            &["name: core", "args: -p lactose-core"],
            &[
                "name: ecs, erased rows and ownership",
                "key: ecs",
                "args: -p lactose-ecs --test erased_rows --test ownership",
            ],
            &[
                "name: ecs, the rest",
                "args: >-",
                "  -p lactose-ecs --lib --test '[!eo]*' --test erased --test events",
                "  --test obligation",
            ],
            &["name: ecs, doc", "args: -p lactose-ecs --doc"],
            &["name: net-host", "args: -p lactose-net-host --test interpolation"],
        ])
    }

    #[test]
    fn the_split_on_main_runs_every_target_once() {
        assert_eq!(run(ecs(&TESTS), &main_split()), [] as [&str; 0]);
        assert_eq!(run(ecs(&TESTS), &main_split().replace('\n', "\r\n")), [] as [&str; 0]);
    }

    #[test]
    fn a_target_no_entry_selects_is_named() {
        let mut tests = TESTS.to_vec();
        tests.push("example");
        assert_eq!(
            run(ecs(&tests), &main_split()),
            [".github/workflows/nightly.yml: the `miri` entries for `lactose-ecs` do not run \
              test target `example`; select it in one of them"]
        );
    }

    #[test]
    fn a_target_two_entries_select_is_named_with_both() {
        let text = main_split().replace("--test events", "--test events --test erased_rows");
        assert_eq!(
            run(ecs(&TESTS), &text),
            [".github/workflows/nightly.yml: the `miri` entries for `lactose-ecs` run test \
              target `erased_rows` 2 times: `ecs, erased rows and ownership` (line 13), `ecs, the \
              rest` (line 16)"]
        );
    }

    #[test]
    fn the_library_and_the_doc_tests_are_each_run_once() {
        let text = main_split().replace(" --lib", "").replace("--doc", "--lib");
        assert_eq!(
            run(ecs(&TESTS), &text),
            [".github/workflows/nightly.yml: the `miri` entries for `lactose-ecs` do not run \
              the doc tests; select it in one of them"]
        );
        let text = main_split().replace("--test ownership", "--test ownership --lib");
        assert_eq!(
            run(ecs(&TESTS), &text),
            [".github/workflows/nightly.yml: the `miri` entries for `lactose-ecs` run the \
              library's unit tests 2 times: `ecs, erased rows and ownership` (line 13), `ecs, the \
              rest` (line 16)"]
        );
        // No target named runs them all, the doc tests included.
        let whole = nightly(&[&["name: ecs", "args: -p lactose-ecs"]]);
        assert_eq!(run(ecs(&TESTS), &whole), [] as [&str; 0]);
        let mut no_doc = ecs(&TESTS);
        no_doc.doctests = false;
        assert_eq!(run(no_doc.clone(), &whole), [] as [&str; 0]);
        let found = run(no_doc, &main_split());
        assert!(
            found[0].ends_with("names `--doc`, but `lactose-ecs` has no doc tests"),
            "{found:?}"
        );
    }

    #[test]
    fn what_cargo_or_the_shell_would_refuse_is_reported() {
        let cases = [
            ("-p lactose-ecs --test nothing", "`--test nothing` selects no test target"),
            ("-p lactose-ecs --test 'n*'", "`--test n*` selects no test target"),
            ("-p lactose-ecs --test 'a**'", "is not a pattern cargo accepts"),
            ("-p lactose-ecs --doc --lib", "mixes `--doc`"),
            ("-p lactose-ecs --bins", "does not know what `--bins` runs"),
            ("-p lactose-ecs -- --skip x", "does not know what `--` runs"),
            ("-p lactose-ecs --test [!eo]*", "an unquoted `[`"),
            ("-p lactose-ecs --test \"$X\"", "a `$`"),
            ("-p lactose-ecs; echo", "an unquoted `;`"),
            ("-p lactose-ecs -p lactose-core", "more than one package"),
        ];
        for (args, message) in cases {
            let found = run(ecs(&TESTS), &nightly(&[&["name: e", &format!("args: {args}")]]));
            assert!(found.iter().any(|f| f.contains(message)), "{args}: {found:?}");
        }
        // An entry for another package is not judged, and the package then has none.
        let found = run(ecs(&TESTS), &nightly(&[&["name: core", "args: -p lactose-core --bins"]]));
        assert_eq!(
            found,
            [".github/workflows/nightly.yml: no entry of the `miri` job runs `lactose-ecs`, \
              which tools/xtask/ci-policy.toml lists under [miri_split]"]
        );
    }

    #[test]
    fn the_reader_reads_the_scalars_the_job_uses_and_refuses_the_rest() {
        let read = |lines: &[&str]| {
            entries(&nightly(&[lines])).map(|e| e[0].args.as_ref().map(|(_, args)| args.clone()))
        };
        let args = |value: &str| Ok(Some(value.to_owned()));
        assert_eq!(read(&["args: -p x  # a comment"]), args("-p x"));
        assert_eq!(read(&["args: -p x", "  --lib"]), args("-p x --lib"));
        assert_eq!(read(&["args: '-p x ''y'''"]), args("-p x 'y'"));
        assert_eq!(read(&["args: \"-p x\" # c"]), args("-p x"));
        assert_eq!(read(&["args: >-", "  -p x", "  --lib"]), args("-p x --lib"));
        assert_eq!(read(&["args: >", "  -p x", "  --lib"]), args("-p x --lib\n"));
        assert_eq!(read(&["args: |-", "  -p x", "  --lib"]), args("-p x\n--lib"));
        assert_eq!(read(&["args: >-", "  -p x", "", "  --lib"]), args("-p x\n--lib"));
        assert_eq!(
            read(&["name: n # c", "args: >-", "  # not a comment"]),
            args("# not a comment")
        );
        assert_eq!(read(&["name: n"]), Ok(None));
        for lines in [
            &["args: [a, b]"][..],
            &["args: *alias"],
            &["args: 'unclosed"],
            &["args: \"a\\tb\""],
            &["args: >2-", "  x"],
            &["args: >-", "  x", "    y"],
            &["args: x", "  # comment", "  y"],
            &["args: x", "args: y"],
            &["args: x", " stray: y"],
        ] {
            assert!(read(lines).is_err(), "{lines:?}");
        }
        // A line break ends the run step's command, so a value holding one is reported.
        let found =
            run(ecs(&TESTS), &nightly(&[&["name: e", "args: |-", "  -p lactose-ecs", "  --lib"]]));
        assert!(found.iter().any(|f| f.contains("a line break")), "{found:?}");
        assert!(entries("jobs:\n  miri:\n    runs-on: x\n").unwrap_err().contains("has no"));
    }

    #[test]
    fn the_checked_in_nightly_workflow_reads() {
        let text = include_str!("../../../.github/workflows/nightly.yml");
        let entries = entries(text).unwrap();
        assert!(entries.iter().all(|e| !e.name.is_empty() && e.args.is_some()), "{entries:?}");
        assert!(entries.iter().any(|e| e.name == "ecs, the rest"), "{entries:?}");
    }
}
