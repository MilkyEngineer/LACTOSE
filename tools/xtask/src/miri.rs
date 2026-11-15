//! The nightly Miri job's split of a package (architecture section 20): the entries of the
//! `miri` job's matrix in `.github/workflows/nightly.yml` that name a package of the policy
//! file's `[miri_split]` must together run its library, each of its integration test targets
//! and its doc tests exactly once. Without the check only a comment keeps a split complete,
//! and a new test target that no entry's patterns select drops out of Miri unseen.
//!
//! The check reads one shape of the job and refuses any other instead of reading it: a matrix
//! of `include` alone, no `if` or `defaults` on the job or `defaults` on the workflow, and one
//! step that uses `matrix.args`, [`RUN`], with only `env` beside its `run`. Each entry names one
//! workspace member with a plain `-p NAME`, and its targets with `--lib`, `--test` and `--doc`
//! (a package outside `[miri_split]` may name none). The job's other steps are not read.
//!
//! The workflow is read as text, by a reader of the YAML the job is written in: block
//! mappings and sequences, plain and quoted scalars on one line, plain scalars over several,
//! and block scalars, with LF or CRLF line breaks and space indentation; a key twice in one
//! mapping on the path to the entries is refused. An entry's `args` are split as the run
//! step's shell splits them, and its `--test` patterns are matched as cargo matches them: a
//! name with none of `*?[]` exactly, any other as a pattern of the `glob` crate (see
//! [`Pattern`]).

use std::collections::BTreeMap;

use crate::policy::Policy;
use crate::repo::Files;
use crate::workspace::{Package, Workspace};
use crate::{Finding, POLICY_FILE};

/// The repository-relative path of the nightly workflow.
pub const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// The keys from the workflow's root to the Miri job's list of matrix entries.
const ENTRIES: [&str; 5] = ["jobs", "miri", "strategy", "matrix", "include"];

/// The run step whose command line the split of an entry's `args` assumes: the words follow
/// `cargo miri test`, and nothing after them selects or skips a test.
const RUN: &str = "- run: cargo +$MIRI_TOOLCHAIN miri test ${{ matrix.args }} --locked";

/// Checks the entries of the Miri job, and the split of every package in `[miri_split]`
/// against its targets.
#[must_use]
pub fn check(policy: &Policy, ws: &Workspace, files: &Files) -> Vec<Finding> {
    let mut findings = Vec::new();
    if policy.miri_split.is_empty() {
        findings.push(Finding::new(
            POLICY_FILE,
            "[miri_split] lists no package, so no split of the nightly Miri job is checked"
                .to_owned(),
        ));
    }
    let mut split = Vec::new();
    for name in &policy.miri_split {
        match ws.package(name) {
            Some(package) => split.push(package),
            None => findings.push(Finding::new(
                POLICY_FILE,
                format!("[miri_split] lists `{name}`, which is no workspace member"),
            )),
        }
    }
    let Some(text) = files.text(NIGHTLY) else {
        findings
            .push(Finding::new(NIGHTLY, "is missing; the check reads its `miri` job".to_owned()));
        return findings;
    };
    let entries = match entries(&text) {
        Ok(entries) => entries,
        Err(message) => {
            findings.push(Finding::new(NIGHTLY, message));
            return findings;
        }
    };
    let mut read = Vec::new();
    for entry in &entries {
        let found = match &entry.args {
            Some((line, args)) => words(args)
                .and_then(|words| selected(ws, policy, &words))
                .map_err(|message| format!("line {line}: the args of `{}`: {message}", entry.name)),
            None => Err(format!(
                "line {}: the entry `{}` has no `args`, so it names no package",
                entry.line, entry.name
            )),
        };
        match found {
            Ok((package, runs)) => read.push((entry, package, runs)),
            Err(message) => findings.push(Finding::new(NIGHTLY, message)),
        }
    }
    // What an entry that was not read runs is unknown, so no count would be true.
    if read.len() < entries.len() {
        return findings;
    }
    unlisted(policy, &read, &mut findings);
    for package in split {
        counts(package, &read, &mut findings);
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

/// An entry that was read: the package it names and what it runs of it, empty when it names
/// no target and so runs the package whole.
type Read<'e, 'w> = (&'e Entry, &'w Package, Vec<Run<'w>>);

/// The finding for each package `[miri_split]` does not list that two or more entries name,
/// one of them by target: a split the check would not count. Entries that each run the
/// package whole, as `lactose-task`'s two do under two environments, are no split.
fn unlisted(policy: &Policy, read: &[Read<'_, '_>], findings: &mut Vec<Finding>) {
    let mut by: BTreeMap<&str, (Vec<&Entry>, bool)> = BTreeMap::new();
    for (entry, package, runs) in read {
        let (entries, targets) = by.entry(package.name.as_str()).or_default();
        entries.push(entry);
        *targets |= !runs.is_empty();
    }
    for (name, (entries, targets)) in by {
        if entries.len() > 1 && targets && !policy.miri_split.iter().any(|split| split == name) {
            findings.push(Finding::new(
                POLICY_FILE,
                format!(
                    "[miri_split] does not list `{name}`, which the `miri` entries {} split by \
                     target; list it so that the check counts its targets",
                    entries.iter().map(|entry| entry.label()).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
}

/// The finding for each target of `package` that `cargo test` runs and no entry can select,
/// and for each of its runs that the entries do not run exactly once.
fn counts(package: &Package, read: &[Read<'_, '_>], findings: &mut Vec<Finding>) {
    for (kind, name) in &package.other_tests {
        findings.push(Finding::new(
            package.manifest(),
            format!(
                "`cargo test` runs the {kind} target `{name}`, which the Miri split of `{}` does \
                 not select; set `test = false` on it",
                package.name
            ),
        ));
    }
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
    for (entry, _, selected) in read.iter().filter(|(_, of, _)| of.name == package.name) {
        named = true;
        for run in selected {
            runs.entry(*run).or_default().push(entry);
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

/// The workspace member that `cargo miri test` with `words` tests, and what it runs of it,
/// sorted: empty when the words name no target. A target two of the words select is run
/// once, as cargo runs it.
///
/// An explicit `--test` runs its target whatever the manifest's `test` says, and fails the
/// job if the target has `required-features` the job does not enable (cargo 1.99.0 and
/// nightly-2026-10-04 both), so neither needs a rule here. Naming no target runs what those
/// keys leave in, which the counts do not model, so a package in `[miri_split]` must name
/// its targets.
///
/// # Errors
/// A package named other than once by a plain `-p NAME` or `--package NAME` with a member's
/// exact name, a word the check does not read, a target cargo would refuse, `--doc` beside
/// another target selection, which cargo refuses, and no target named in a split package.
fn selected<'w>(
    ws: &'w Workspace,
    policy: &Policy,
    words: &[String],
) -> Result<(&'w Package, Vec<Run<'w>>), String> {
    fn value<'v>(flag: &str, words: &mut impl Iterator<Item = &'v str>) -> Result<&'v str, String> {
        words.next().ok_or_else(|| format!("`{flag}` has no value"))
    }
    let (mut package, mut lib, mut doc, mut patterns) = (None, false, false, Vec::new());
    let mut words = words.iter().map(String::as_str);
    while let Some(word) = words.next() {
        match word {
            "-p" | "--package" => {
                let name = value(word, &mut words)?;
                if package.is_some() {
                    return Err("names a package twice; an entry names one".to_owned());
                }
                package = Some(ws.package(name).ok_or_else(|| {
                    format!("`{word} {name}` names no workspace member by its exact name")
                })?);
            }
            "--lib" => lib = true,
            "--doc" => doc = true,
            "--test" => patterns.push(value(word, &mut words)?),
            _ if word.starts_with("--test=") => patterns.push(&word["--test=".len()..]),
            _ if word.starts_with("-p") || word.starts_with("--package") => {
                return Err(format!("`{word}`: the check reads a package as `-p NAME` alone"));
            }
            _ => return Err(format!("the check does not know what `{word}` runs")),
        }
    }
    let package = package.ok_or("names no package with `-p`, which the check reads")?;
    if doc && (lib || !patterns.is_empty()) {
        return Err("mixes `--doc` with another target selection, which cargo refuses".to_owned());
    }
    if doc {
        if !package.doctests {
            return Err(format!("names `--doc`, but `{}` has no doc tests", package.name));
        }
        return Ok((package, vec![Run::Doc]));
    }
    if !lib && patterns.is_empty() {
        if policy.miri_split.contains(&package.name) {
            return Err(format!(
                "names no target of `{}`, which [miri_split] lists; name its targets with \
                 `--lib`, `--test` and `--doc`",
                package.name
            ));
        }
        return Ok((package, Vec::new()));
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
    runs.sort();
    runs.dedup();
    Ok((package, runs))
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

/// A `--test` pattern, matched as cargo 1.99 and the Miri job's cargo 1.101.0-nightly
/// (nightly-2026-10-04) match it: with `glob::Pattern` and its default match options, so
/// case-sensitively against the whole target name. `?` is one character, `*` any run of
/// them, `**` is `*` but only as the whole pattern, and `[...]` and `[!...]` are a class and
/// its complement: a `]` right after the opening is a member, and `a-z` is a range unless the
/// `-` is first or last. No character escapes another.
///
/// The cases of the unit tests below were each run through `cargo test --no-run --test` on
/// cargo 1.99.0, those of `review_r1_cargo_probe_selections` on both cargos, and the
/// selections and refusals are cargo's. A pattern holding a path separator is refused here,
/// since `glob` treats `/` (and on Windows `\`) as one.
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
/// When the workflow has no such list, the list holds YAML the reader does not read, or the
/// job around it could run the entries other than as [`RUN`] does; the message names the
/// line.
fn entries(text: &str) -> Result<Vec<Entry>, String> {
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let unread = |what: &str| format!("line {number}: {what}, which the check does not read");
        // YAML breaks a line at a lone CR, and libyaml also at NEL, LS and PS, which `lines`
        // does not: a comment could hide a line from the reader.
        if raw.contains(['\r', '\u{85}', '\u{2028}', '\u{2029}']) {
            return Err(unread("a line break other than LF or CRLF"));
        }
        let text = raw.trim_start_matches(' ');
        if text.starts_with('\t') {
            return Err(unread("a tab in the indentation"));
        }
        let marker = |marker: &str| {
            raw.strip_prefix(marker)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        };
        if raw.starts_with('%') || marker("---") || marker("...") {
            return Err(unread("a document marker or directive"));
        }
        lines.push(Line { number, indent: raw.len() - text.len(), text: text.trim_end() });
    }
    if let Some((line, _)) = keys(&lines).find(|(_, key)| *key == "defaults") {
        return Err(format!(
            "line {line}: the workflow's `defaults`, which could give the run step another shell \
             and the check does not read"
        ));
    }
    let mut body = lines.as_slice();
    let mut miri = body;
    for key in ENTRIES {
        body = child(body, key)?
            .ok_or_else(|| format!("has no `{}` list for the check to read", ENTRIES.join(".")))?;
        match key {
            "miri" => miri = body,
            // Beside another key, GitHub adds each entry to every combination it makes, and a
            // later entry's values replace an earlier one's: the entries would not each run.
            "matrix" => {
                if let Some((line, key)) = keys(body).find(|(_, key)| *key != "include") {
                    return Err(format!(
                        "line {line}: the matrix has `{key}` beside `include`, which folds the \
                         entries into its combinations"
                    ));
                }
            }
            _ => {}
        }
    }
    job(miri)?;
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

/// The line number and key of each significant line of the mapping `body` holds, at the
/// indent of its first significant line, without the quotes of a quoted key.
fn keys<'b>(body: &'b [Line<'_>]) -> impl Iterator<Item = (usize, &'b str)> {
    let indent = body.iter().find(|line| line.significant()).map(|line| line.indent);
    body.iter().filter(move |line| line.significant() && Some(line.indent) == indent).map(|line| {
        let key = line.text.split(':').next().unwrap_or_default().trim_end();
        let unquoted = ['"', '\''].iter().find_map(|q| key.strip_prefix(*q)?.strip_suffix(*q));
        (line.number, unquoted.unwrap_or(key))
    })
}

/// Refuses what in the Miri job, outside its matrix, could change what an entry runs: an `if`
/// or `defaults` on the job, and any use of `matrix.args` but the [`RUN`] step, whose other
/// keys are `env`.
fn job(body: &[Line<'_>]) -> Result<(), String> {
    if let Some((line, key)) = keys(body).find(|(_, key)| matches!(*key, "if" | "defaults")) {
        return Err(format!(
            "line {line}: the `miri` job's `{key}`, which the check does not read"
        ));
    }
    let uses: Vec<usize> = (0..body.len())
        .filter(|&at| {
            body[at].significant()
                && (body[at].text.contains("matrix.args") || body[at].text.contains("matrix["))
        })
        .collect();
    let [at] = uses[..] else {
        return Err(format!(
            "the `miri` job uses `matrix.args` {} times; the check reads one",
            uses.len()
        ));
    };
    let run = body[at];
    if run.text != RUN {
        return Err(format!(
            "line {}: the step that uses `matrix.args` is not `{RUN}`, which the check reads",
            run.number
        ));
    }
    let step = &body[at + 1..];
    let step = &step[..block_end(step, run.indent).unwrap_or(step.len())];
    if let Some((line, key)) = keys(step).find(|(_, key)| *key != "env") {
        return Err(format!("line {line}: the run step's `{key}`, which the check does not read"));
    }
    Ok(())
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
///
/// # Errors
/// A second `key` in the mapping: YAML readers differ on which of the two they keep.
fn child<'a, 'b>(body: &'b [Line<'a>], key: &str) -> Result<Option<&'b [Line<'a>]>, String> {
    let mut found = keys(body).filter(|(_, found)| *found == key);
    if let (Some((first, _)), Some((second, _))) = (found.next(), found.next()) {
        return Err(format!(
            "lines {first} and {second}: `{key}` twice in one mapping, which the check does not \
             read"
        ));
    }
    let Some(indent) = body.iter().find(|line| line.significant()).map(|line| line.indent) else {
        return Ok(None);
    };
    let start = body.iter().position(|line| {
        line.significant()
            && line.indent == indent
            && line.text.strip_prefix(key).and_then(|rest| rest.strip_prefix(':')).is_some_and(
                |rest| {
                    let rest = rest.trim_start();
                    rest.is_empty() || rest.starts_with('#')
                },
            )
    });
    Ok(start.map(|start| {
        let rest = &body[start + 1..];
        &rest[..block_end(rest, indent).unwrap_or(rest.len())]
    }))
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

    /// The findings for `nightly` in a workspace of `ecs` and the other members the main
    /// split names.
    fn run(ecs: Package, nightly: &str) -> Vec<String> {
        run_with(&policy(), ecs, nightly)
    }

    fn run_with(policy: &Policy, ecs: Package, nightly: &str) -> Vec<String> {
        let mut net_host = package("lactose-net-host", &[]);
        net_host.tests = vec!["interpolation".to_owned()];
        let members =
            vec![package("lactose-core", &[]), ecs, net_host, package("lactose-task", &[])];
        let mut files = Files::default();
        files.insert(NIGHTLY, nightly);
        check(policy, &workspace(members), &files).iter().map(ToString::to_string).collect()
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
        text.push_str("    steps:\n      ");
        text.push_str(RUN);
        text.push_str("\n  loom:\n    name: x\n");
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
        // One entry that selects a target twice runs it once (cargo 1.99.0, `--test a --test
        // '[ab]'`).
        let text = main_split().replace("--test events", "--test events --test 'ev*'");
        assert_eq!(run(ecs(&TESTS), &text), [] as [&str; 0]);
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
        let mut no_doc = ecs(&TESTS);
        no_doc.doctests = false;
        let found = run(no_doc, &main_split());
        assert!(
            found[0].ends_with("names `--doc`, but `lactose-ecs` has no doc tests"),
            "{found:?}"
        );
    }

    #[test]
    fn a_split_package_names_its_targets() {
        // Naming no target runs what the manifest's `test` and `required-features` leave in,
        // which the counts do not model.
        let whole = nightly(&[&["name: ecs", "args: -p lactose-ecs"]]);
        assert_eq!(
            run(ecs(&TESTS), &whole),
            [".github/workflows/nightly.yml: line 12: the args of `ecs`: names no target of \
              `lactose-ecs`, which [miri_split] lists; name its targets with `--lib`, `--test` \
              and `--doc`"]
        );
        // A target `cargo test` runs that the split cannot select is named in its manifest.
        let mut with_bin = ecs(&TESTS);
        with_bin.other_tests = vec![("bin".to_owned(), "tool".to_owned())];
        assert_eq!(
            run(with_bin, &main_split()),
            ["crates/ecs/Cargo.toml: `cargo test` runs the bin target `tool`, which the Miri \
              split of `lactose-ecs` does not select; set `test = false` on it"]
        );
        // An explicit `--test` runs a `test = false` target, and fails the job on one whose
        // `required-features` it does not enable (cargo 1.99.0 and nightly-2026-10-04, the
        // review's probes): each is counted as run, and needs no rule.
    }

    #[test]
    fn an_entry_names_one_member_plainly() {
        let cases = [
            ("--workspace --lib", "the check does not know what `--workspace` runs"),
            ("--lib", "names no package with `-p`"),
            ("-p lactose-ecs -p lactose-ecs --lib", "names a package twice"),
            ("-p lactose-ecs -p lactose-core --lib", "names a package twice"),
            ("--package lactose-ecs --lib -p lactose-core", "names a package twice"),
            ("-p=lactose-ecs --lib", "`-p=lactose-ecs`: the check reads a package as `-p NAME`"),
            ("-plactose-ecs --lib", "`-plactose-ecs`: the check reads"),
            ("--package=lactose-ecs --lib", "`--package=lactose-ecs`: the check reads"),
            ("-p lactose-ecs@0.0.0 --lib", "`-p lactose-ecs@0.0.0` names no workspace member"),
            ("-p 'lactose-e*' --lib", "`-p lactose-e*` names no workspace member"),
            ("-p", "`-p` has no value"),
        ];
        for (args, message) in cases {
            let text = main_split().replace(
                "- name: core\n",
                &format!("- name: extra\n            args: {args}\n          - name: core\n"),
            );
            let found = run(ecs(&TESTS), &text);
            assert!(
                found.len() == 1
                    && found[0].contains(&format!("line 12: the args of `extra`: {message}")),
                "{args}: {found:?}"
            );
        }
        let text =
            main_split().replace("- name: core\n", "- name: extra\n          - name: core\n");
        assert_eq!(
            run(ecs(&TESTS), &text),
            [".github/workflows/nightly.yml: line 11: the entry `extra` has no `args`, so it \
              names no package"]
        );
        // `lactose-task` runs whole twice, under two environments: not a split.
        let text = main_split().replace(
            "- name: core\n",
            "- name: task\n            args: -p lactose-task\n          - name: task 2\n\
             \x20           args: -p lactose-task\n          - name: core\n",
        );
        assert_eq!(run(ecs(&TESTS), &text), [] as [&str; 0]);
    }

    #[test]
    fn a_package_split_outside_the_policy_is_named() {
        let text = main_split().replace(
            "- name: core\n",
            "- name: core\n            args: -p lactose-core\n          - name: core lib\n\
             \x20           args: -p lactose-core --lib\n          - name: other\n",
        );
        assert_eq!(
            run(ecs(&TESTS), &text),
            ["tools/xtask/ci-policy.toml: [miri_split] does not list `lactose-core`, which the \
              `miri` entries `core` (line 11), `core lib` (line 13), `other` (line 15) split by \
              target; list it so that the check counts its targets"]
        );
        let empty = Policy::default();
        assert_eq!(
            run_with(&empty, ecs(&TESTS), &main_split()),
            [
                "tools/xtask/ci-policy.toml: [miri_split] lists no package, so no split of the \
                 nightly Miri job is checked",
                "tools/xtask/ci-policy.toml: [miri_split] does not list `lactose-ecs`, which the \
                 `miri` entries `ecs, erased rows and ownership` (line 13), `ecs, the rest` \
                 (line 16), `ecs, doc` (line 20) split by target; list it so that the check \
                 counts its targets"
            ]
        );
        let stray = Policy { miri_split: vec!["lactose-nothing".to_owned()], ..Policy::default() };
        assert!(
            run_with(&stray, ecs(&TESTS), &main_split()).contains(
                &"tools/xtask/ci-policy.toml: [miri_split] lists `lactose-nothing`, which is \
                        no workspace member"
                    .to_owned()
            )
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
            ("-p lactose-core --test none", "`--test none` selects no test target"),
        ];
        for (args, message) in cases {
            let found = run(ecs(&TESTS), &nightly(&[&["name: e", &format!("args: {args}")]]));
            assert!(found.len() == 1 && found[0].contains(message), "{args}: {found:?}");
        }
        // The package then has no entry.
        let found = run(ecs(&TESTS), &nightly(&[&["name: core", "args: -p lactose-core"]]));
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
    fn review_r1_what_github_would_run_differently_is_refused() {
        let base = main_split();
        let cases = [
            // GitHub folds every entry into the one combination; the last one's args win.
            base.replace("        include:\n", "        toolchain: [dated]\n        include:\n"),
            base.replace("--locked\n", "--locked -- --skip erased\n"),
            base.replace(
                "      - run: cargo",
                "      - if: matrix.key != 'ecs-rest'\n        run: cargo",
            ),
            base.replace("    name: miri\n", "    name: miri\n    if: false\n"),
            base.replace(
                "        include:\n",
                "        # c\r          - name: hidden\n        include:\n",
            ),
            base.replace("        include:\n", "        # c\u{2028}x: y\n        include:\n"),
            base.replace("          - name: core\n", "\t# c\n          - name: core\n"),
        ];
        for text in cases {
            let found = run(ecs(&TESTS), &text);
            assert!(found.len() == 1 && found[0].contains("nightly.yml: "), "{text}: {found:?}");
        }
    }

    #[test]
    fn a_job_or_file_of_another_shape_is_refused_at_its_line() {
        const NAME: &str = "name: nightly\n";
        const LOOM: &str = "  loom:\n";
        const INCLUDE: &str = "        include:\n";
        const JOB: &str = "    name: miri\n";
        let hidden = "        include:\n          - name: hidden\n            args: -p lactose-ecs \
                      --lib\n        include:\n";
        let job_defaults = "    name: miri\n    defaults:\n      run:\n        shell: sh\n";
        let cases = [
            (NAME, "---\nname: nightly\n", "line 1: a document marker"),
            (LOOM, "...\n---\njobs:\n  loom:\n", "line 26: a document marker"),
            (NAME, "%YAML 1.2\nname: nightly\n", "line 1: a document marker"),
            (INCLUDE, hidden, "lines 10 and 13: `include` twice in one mapping"),
            (LOOM, "  miri:\n    name: y\n  loom:\n", "lines 5 and 26: `miri` twice"),
            (LOOM, "  'miri':\n    name: y\n  loom:\n", "lines 5 and 26: `miri` twice"),
            ("\njobs:\n", "\njobs:\n  a:\n    name: a\njobs: {}\n", "lines 3 and 6: `jobs` twice"),
            ("\njobs:\n", "\ndefaults:\n  run:\n    shell: sh\njobs:\n", "line 3: the workflow's"),
            (JOB, job_defaults, "line 7: the `miri` job's `defaults`"),
            (JOB, "    name: miri\n    \"if\": false\n", "line 7: the `miri` job's `if`"),
            ("    steps:\n", "    env:\n      A: ${{ matrix['args'] }}\n    steps:\n", "2 times"),
            ("          - name: core\n", "\t  - name: core\n", "line 11: a tab in the indentation"),
            (INCLUDE, "        include: # c\rd\n", "line 10: a line break other than LF or CRLF"),
            (INCLUDE, "        include: # c\u{85}\n", "line 10: a line break other than LF"),
        ];
        for (old, new, message) in cases {
            let found = run(ecs(&TESTS), &main_split().replace(old, new));
            assert!(
                found.len() == 1
                    && found[0].starts_with(".github/workflows/nightly.yml: ")
                    && found[0].contains(message),
                "{message}: {found:?}"
            );
        }
    }

    #[test]
    fn review_r1_cargo_probe_selections() {
        // `cargo test --test <pattern>` on cargo 1.99.0 and on nightly-2026-10-04, which agree.
        const PROBED: [&str; 17] = [
            "Cap",
            "a",
            "ab",
            "b",
            "e1",
            "erased",
            "erased_rows",
            "needsx",
            "noharness",
            "notest",
            "o",
            "ownership",
            "x-y",
            "z",
            "\u{e9}",
            "\u{e9}a",
            "\u{65e5}\u{672c}",
        ];
        let select = |text: &str| -> Vec<&str> {
            let pattern = Pattern::new(text).unwrap();
            PROBED.iter().copied().filter(|name| pattern.matches(name)).collect()
        };
        let all: Vec<&str> = PROBED.to_vec();
        assert_eq!(select("[]]"), [] as [&str; 0]);
        assert_eq!(select("[!]]"), ["a", "b", "o", "z", "\u{e9}"]);
        assert_eq!(select("[a-]"), ["a"]);
        assert_eq!(select("[-a]"), ["a"]);
        assert_eq!(select("[z-a]"), [] as [&str; 0]);
        assert_eq!(select("*"), all);
        assert_eq!(select("**"), all);
        assert_eq!(select("?"), ["a", "b", "o", "z", "\u{e9}"]);
        assert_eq!(select("??"), ["ab", "e1", "\u{e9}a", "\u{65e5}\u{672c}"]);
        assert_eq!(select("[\u{e9}]*"), ["\u{e9}", "\u{e9}a"]);
        assert_eq!(select("[!a-z]*"), ["Cap", "\u{e9}", "\u{e9}a", "\u{65e5}\u{672c}"]);
        assert_eq!(select("[!a-z\u{e9}]*"), ["Cap", "\u{65e5}\u{672c}"]);
        assert_eq!(select("\u{65e5}?"), ["\u{65e5}\u{672c}"]);
        assert_eq!(select("[[]"), [] as [&str; 0]);
        assert_eq!(
            select("[!eo]*"),
            [
                "Cap",
                "a",
                "ab",
                "b",
                "needsx",
                "noharness",
                "notest",
                "x-y",
                "z",
                "\u{e9}",
                "\u{e9}a",
                "\u{65e5}\u{672c}"
            ]
        );
        assert_eq!(select("n*"), ["needsx", "noharness", "notest"]);
        assert!(!Pattern::is_pattern("\u{e9}"));
    }

    #[test]
    fn the_checked_in_nightly_workflow_reads() {
        let text = include_str!("../../../.github/workflows/nightly.yml");
        let entries = entries(text).unwrap();
        assert!(entries.iter().all(|e| !e.name.is_empty() && e.args.is_some()), "{entries:?}");
        assert!(entries.iter().any(|e| e.name == "ecs, the rest"), "{entries:?}");
    }
}
