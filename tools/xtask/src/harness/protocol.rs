//! The contract between the SOUR mode runner and a simulation test (architecture section
//! 8.3): environment schema version 1, record format version 2.
//!
//! This module depends on nothing but `std` and on nothing else in this crate, so that it can
//! move unchanged into the library the harness test crate will depend on; `README.md` records
//! where it is meant to go.
//!
//! # How a test learns its mode
//!
//! Layout and chaos modes are runtime options on the world and executor builders, never
//! `cfg(test)` or Cargo features. The runner therefore passes one [`RunConfig`] to the test
//! process in environment variables, and the test reads it once with [`Session::from_env`]
//! and hands the values to its builders.
//!
//! | Variable | Value | Meaning |
//! |---|---|---|
//! | `LACTOSE_SOUR_SCHEMA` | `1` | The version of this table. Absent: the test is not running under the runner, and every other variable is ignored |
//! | `LACTOSE_SOUR_WORKERS` | `default` or a count from 1 | Frame-critical workers. `default` leaves the executor builder's choice alone |
//! | `LACTOSE_SOUR_LAYOUT` | `production`, `rows:<n>:<seed>` or `bytes:<n>:<seed>` | Chunk capacity as a row count or a slab size in bytes. The seed drives row permutation at every sync point, chunk iteration order and batch splits |
//! | `LACTOSE_SOUR_CHAOS` | `off` or a seed | The seed of chaos scheduling: seeded steal order, injected yields, forced split-on-steal, with the execution log recorded |
//! | `LACTOSE_SOUR_HASH_FILE` | a path, optional | Where [`Recorder`] appends its records |
//! | `LACTOSE_SOUR_LOG_DIR` | a path, optional | The directory a chaos run writes its execution log into |
//!
//! With the schema variable set, the three configuration variables are required and a value
//! that does not parse is an error. A test must fail on that error and never fall back to
//! the production configuration, or a mode would pass without having run.
//!
//! # Records
//!
//! The hash file is lines of tab-separated fields; the first field says what the line is.
//!
//! | Line | Fields after the first |
//! |---|---|
//! | `applied` | test name, then workers, layout and chaos in the spelling of the variables above |
//! | `hash` | test name, tick, world hash as sixteen lowercase hexadecimal digits |
//!
//! A line that starts with `#` is a comment.
//!
//! [`Session::recorder`] takes the configuration the test **applied**: what its world and
//! executor builders report they were built with, not a copy of what the session asked for.
//! The recorder writes it as an `applied` line before any hash, and the runner fails a run
//! whose tests applied anything other than what it launched. A test that records hashes while
//! ignoring its configuration is therefore caught, provided the builders report honestly.
//!
//! The file is opened for appending and every line is a single write, because nextest runs
//! each test in its own process and they all append to the one file the runner named.
//!
//! # Execution logs
//!
//! A test that runs under a chaos seed writes the bytes of its `lactose_task::ExecutionLog`
//! with [`Session::write_execution_log`] on every run, pass or fail: a divergence is only
//! known once the runner has compared hashes, after the test process has gone. The runner
//! deletes the logs of runs that agreed and keeps the rest for CI to upload, so the diverging
//! schedule replays exactly.

// This file is compiled into the tests of boundary crates, under their `clippy.toml`, which
// bans `std::fs` and `std::env` (architecture section 8.2), and into this crate and the tests
// of `lactose-net-host`, `lactose-persist` and `lactose-app`, which do not. Each item that
// reaches a file or the environment expects the lint for itself; the `mod protocol`
// declarations outside the boundary allow the expectations that are unfulfilled there.

use std::fmt;
#[expect(
    clippy::disallowed_types,
    reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
)]
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

/// The version of the environment schema.
pub const SCHEMA_VERSION: u32 = 1;
/// The version of the record format.
pub const RECORD_VERSION: u32 = 2;

/// Names the schema version; its presence means the test runs under the runner.
pub const ENV_SCHEMA: &str = "LACTOSE_SOUR_SCHEMA";
/// Carries [`RunConfig::workers`].
pub const ENV_WORKERS: &str = "LACTOSE_SOUR_WORKERS";
/// Carries [`RunConfig::layout`].
pub const ENV_LAYOUT: &str = "LACTOSE_SOUR_LAYOUT";
/// Carries [`RunConfig::chaos_seed`].
pub const ENV_CHAOS: &str = "LACTOSE_SOUR_CHAOS";
/// Names the file records are appended to.
pub const ENV_HASH_FILE: &str = "LACTOSE_SOUR_HASH_FILE";
/// Names the directory execution logs are written into.
pub const ENV_LOG_DIR: &str = "LACTOSE_SOUR_LOG_DIR";

/// A violation of the protocol: a malformed variable, record or file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProtocolError(String);

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

/// How much a test chunk holds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Capacity {
    /// A fixed number of rows, in a slab sized to match.
    Rows(u32),
    /// A slab of this many bytes; the row count follows from the row size.
    Bytes(u32),
}

/// The storage layout a world is built with.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum Layout {
    /// The production layout: the builder's defaults, nothing permuted.
    #[default]
    Production,
    /// Layout mode: a test capacity, with rows permuted at every sync point, chunk iteration
    /// reversed and shuffled and batches split at random, all drawn from `seed`.
    Perturbed {
        /// The chunk capacity.
        capacity: Capacity,
        /// The seed of every permutation.
        seed: u64,
    },
}

/// What a mode varies. The runner sends one per test process; the test applies it to its
/// world and executor builders.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct RunConfig {
    /// The number of frame-critical workers, or `None` for the executor builder's default.
    pub workers: Option<NonZeroU32>,
    /// The storage layout.
    pub layout: Layout,
    /// The seed of chaos scheduling, or `None` for free scheduling.
    pub chaos_seed: Option<u64>,
}

impl RunConfig {
    /// The values of the workers, layout and chaos variables for this configuration.
    fn fields(&self) -> [String; 3] {
        let workers = self.workers.map_or_else(|| "default".to_owned(), |n| n.to_string());
        let layout = match self.layout {
            Layout::Production => "production".to_owned(),
            Layout::Perturbed { capacity, seed } => match capacity {
                Capacity::Rows(n) => format!("rows:{n}:{seed}"),
                Capacity::Bytes(n) => format!("bytes:{n}:{seed}"),
            },
        };
        let chaos = self.chaos_seed.map_or_else(|| "off".to_owned(), |seed| seed.to_string());
        [workers, layout, chaos]
    }

    /// The inverse of [`RunConfig::fields`]. The error names the field that is wrong.
    fn from_fields(workers: &str, layout: &str, chaos: &str) -> Result<Self, ProtocolError> {
        let bad = |name: &str, value: &str| {
            ProtocolError(format!("{name} has the unrecognised value `{value}`"))
        };
        let parsed_workers = match workers {
            "default" => None,
            count => Some(count.parse().map_err(|_| bad(ENV_WORKERS, workers))?),
        };
        let parsed_layout = if layout == "production" {
            Layout::Production
        } else {
            let fields: Vec<&str> = layout.split(':').collect();
            let [unit, amount, seed] = fields[..] else {
                return Err(bad(ENV_LAYOUT, layout));
            };
            let amount: u32 = amount.parse().map_err(|_| bad(ENV_LAYOUT, layout))?;
            let capacity = match unit {
                "rows" if amount > 0 => Capacity::Rows(amount),
                "bytes" if amount > 0 => Capacity::Bytes(amount),
                _ => return Err(bad(ENV_LAYOUT, layout)),
            };
            Layout::Perturbed { capacity, seed: seed.parse().map_err(|_| bad(ENV_LAYOUT, layout))? }
        };
        let chaos_seed = match chaos {
            "off" => None,
            seed => Some(seed.parse().map_err(|_| bad(ENV_CHAOS, chaos))?),
        };
        Ok(Self { workers: parsed_workers, layout: parsed_layout, chaos_seed })
    }

    /// The environment variables that carry this configuration, schema version included.
    #[must_use]
    pub fn to_vars(&self) -> Vec<(&'static str, String)> {
        let [workers, layout, chaos] = self.fields();
        vec![
            (ENV_SCHEMA, SCHEMA_VERSION.to_string()),
            (ENV_WORKERS, workers),
            (ENV_LAYOUT, layout),
            (ENV_CHAOS, chaos),
        ]
    }
}

impl fmt::Display for RunConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [workers, layout, chaos] = self.fields();
        write!(f, "workers={workers} layout={layout} chaos={chaos}")
    }
}

/// What one test process was told by the runner: the configuration to apply and where its
/// output goes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Session {
    config: RunConfig,
    hash_file: Option<PathBuf>,
    log_dir: Option<PathBuf>,
}

impl Session {
    /// Reads the session from the process environment. Outside the runner this is the
    /// production configuration with nothing recorded, so a simulation test also runs under
    /// a plain `cargo nextest run`.
    ///
    /// # Errors
    /// If the runner's variables are present but malformed or of another schema version.
    #[expect(
        clippy::disallowed_methods,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    pub fn from_env() -> Result<Self, ProtocolError> {
        Self::from_vars(&|name| std::env::var(name).ok())
    }

    /// [`Session::from_env`] over any source of variables.
    ///
    /// # Errors
    /// As [`Session::from_env`].
    pub fn from_vars(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, ProtocolError> {
        let Some(schema) = lookup(ENV_SCHEMA) else {
            return Ok(Self::default());
        };
        if schema != SCHEMA_VERSION.to_string() {
            return Err(ProtocolError(format!(
                "{ENV_SCHEMA} is `{schema}` but this test speaks version {SCHEMA_VERSION}; \
                 the runner and the test were built from different revisions"
            )));
        }
        let required = |name: &str| {
            lookup(name)
                .ok_or_else(|| ProtocolError(format!("{ENV_SCHEMA} is set but {name} is not")))
        };
        let (workers, layout, chaos) =
            (required(ENV_WORKERS)?, required(ENV_LAYOUT)?, required(ENV_CHAOS)?);
        Ok(Self {
            config: RunConfig::from_fields(&workers, &layout, &chaos)?,
            hash_file: lookup(ENV_HASH_FILE).map(PathBuf::from),
            log_dir: lookup(ENV_LOG_DIR).map(PathBuf::from),
        })
    }

    /// The configuration to build the world and the executor with.
    #[must_use]
    pub fn config(&self) -> &RunConfig {
        &self.config
    }

    /// Opens the recorder for the test called `test`, which must be the same string in every
    /// configuration and on every platform: the full test path is the obvious choice.
    ///
    /// `applied` is the configuration the test's world and executor were actually built
    /// with, as the builders report it. It is recorded for the runner to check, so it must
    /// not be a copy of [`Session::config`].
    ///
    /// # Errors
    /// If the name contains a tab or a line break, or the hash file cannot be written.
    #[expect(
        clippy::disallowed_types,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    pub fn recorder(&self, test: &str, applied: &RunConfig) -> Result<Recorder, ProtocolError> {
        check_test_name(test)?;
        let file = match &self.hash_file {
            None => None,
            Some(path) => Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|e| ProtocolError(format!("{}: {e}", path.display())))?,
            ),
        };
        let mut recorder = Recorder { test: test.to_owned(), file };
        recorder.write(&Line::Applied { test: test.to_owned(), config: *applied })?;
        Ok(recorder)
    }

    /// Where the test called `test` writes its execution log, or `None` when the runner
    /// keeps no logs.
    #[must_use]
    pub fn execution_log_path(&self, test: &str) -> Option<PathBuf> {
        self.log_dir.as_deref().map(|dir| execution_log_path(dir, test))
    }

    /// Writes the execution log of the test called `test`. A test under a chaos seed calls
    /// this on every run. Outside the runner it does nothing.
    ///
    /// # Errors
    /// If the log cannot be written.
    #[expect(
        clippy::disallowed_methods,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    pub fn write_execution_log(&self, test: &str, log: &[u8]) -> Result<(), ProtocolError> {
        let Some(path) = self.execution_log_path(test) else {
            return Ok(());
        };
        let written = match path.parent() {
            Some(dir) => fs::create_dir_all(dir).and_then(|()| fs::write(&path, log)),
            None => fs::write(&path, log),
        };
        written.map_err(|e| ProtocolError(format!("{}: {e}", path.display())))
    }
}

/// The path of a test's execution log inside a log directory. Lowercase ASCII letters,
/// digits, `.` and `-` are kept; every other byte of the name, `_` and uppercase letters
/// included, becomes `_` and two hexadecimal digits. Two different test names therefore
/// never share a file, on a case-insensitive file system either.
#[must_use]
pub fn execution_log_path(log_dir: &Path, test: &str) -> PathBuf {
    let mut name = String::new();
    for byte in test.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-') {
            name.push(char::from(byte));
        } else {
            name.push_str(&format!("_{byte:02x}"));
        }
    }
    log_dir.join(format!("{name}.execlog"))
}

fn check_test_name(test: &str) -> Result<(), ProtocolError> {
    if test.is_empty() || test.starts_with('#') || test.contains(['\t', '\n', '\r']) {
        return Err(ProtocolError(format!(
            "`{}` cannot name a test in a hash record",
            test.escape_debug()
        )));
    }
    Ok(())
}

/// Appends one test's records to the file the runner named, or discards them outside the
/// runner.
#[derive(Debug)]
#[expect(
    clippy::disallowed_types,
    reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
)]
pub struct Recorder {
    test: String,
    file: Option<File>,
}

impl Recorder {
    /// Records the world hash at `tick`, the record's position in the test's sequence (see
    /// [`Record::tick`]).
    ///
    /// # Errors
    /// If the record cannot be written.
    pub fn record(&mut self, tick: u32, hash: u64) -> Result<(), ProtocolError> {
        self.write(&Line::Hash(Record { test: self.test.clone(), tick, hash }))
    }

    fn write(&mut self, line: &Line) -> Result<(), ProtocolError> {
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        // One write per line: appends from concurrent test processes must not interleave.
        file.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| ProtocolError(format!("cannot append a record: {e}")))
    }
}

/// One world hash.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Record {
    /// The test that produced the hash.
    pub test: String,
    /// The record's position in the test's sequence (a simulation tick for a straight run;
    /// a test whose ticks repeat, such as one that rolls back, keys its records by a
    /// monotonic sequence of its own).
    pub tick: u32,
    /// The world hash.
    pub hash: u64,
}

/// One line of a hash file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Line {
    /// The configuration a test says it applied.
    Applied {
        /// The test.
        test: String,
        /// What its builders reported.
        config: RunConfig,
    },
    /// A world hash.
    Hash(Record),
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Applied { test, config } => {
                let [workers, layout, chaos] = config.fields();
                write!(f, "applied\t{test}\t{workers}\t{layout}\t{chaos}")
            }
            Self::Hash(record) => {
                write!(f, "hash\t{}\t{}\t{:016x}", record.test, record.tick, record.hash)
            }
        }
    }
}

/// Parses the text of a hash file. Blank lines and `#` comments are skipped.
///
/// # Errors
/// If a line is neither kind of record. The message names the line.
pub fn parse_lines(text: &str) -> Result<Vec<Line>, ProtocolError> {
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let parsed = match fields[..] {
            ["applied", test, workers, layout, chaos] if check_test_name(test).is_ok() => {
                RunConfig::from_fields(workers, layout, chaos)
                    .ok()
                    .map(|config| Line::Applied { test: test.to_owned(), config })
            }
            // `parse` and `from_str_radix` accept a leading sign; the format does not.
            ["hash", test, tick, hash]
                if hash.len() == 16
                    && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                    && tick.bytes().all(|b| b.is_ascii_digit())
                    && check_test_name(test).is_ok() =>
            {
                tick.parse()
                    .ok()
                    .zip(u64::from_str_radix(hash, 16).ok())
                    .map(|(tick, hash)| Line::Hash(Record { test: test.to_owned(), tick, hash }))
            }
            _ => None,
        };
        match parsed {
            Some(parsed) => lines.push(parsed),
            None => {
                return Err(ProtocolError(format!(
                    "line {} is not a version {RECORD_VERSION} record: `{line}`",
                    index + 1
                )));
            }
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&'static str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone())
    }

    fn session(vars: &[(&'static str, &str)]) -> Result<Session, ProtocolError> {
        let owned: Vec<(&'static str, String)> =
            vars.iter().map(|&(n, v)| (n, v.to_owned())).collect();
        Session::from_vars(&lookup(&owned))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lactose-harness-protocol-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn outside_the_runner_a_test_gets_the_production_configuration() {
        // The other variables are ignored without the schema variable.
        let session = session(&[(ENV_WORKERS, "3"), (ENV_HASH_FILE, "x")]).unwrap();
        assert_eq!(session, Session::default());
        assert_eq!(*session.config(), RunConfig::default());
        assert_eq!(session.execution_log_path("t"), None);
        session.write_execution_log("t", b"log").unwrap();
        let mut recorder = session.recorder("t", &RunConfig::default()).unwrap();
        recorder.record(0, 1).unwrap();
    }

    #[test]
    fn every_configuration_round_trips_through_its_variables() {
        let configs = [
            RunConfig::default(),
            RunConfig {
                workers: NonZeroU32::new(4),
                layout: Layout::Perturbed { capacity: Capacity::Rows(7), seed: u64::MAX },
                chaos_seed: Some(0),
            },
            RunConfig {
                workers: NonZeroU32::new(1),
                layout: Layout::Perturbed { capacity: Capacity::Bytes(16 * 1024), seed: 3 },
                chaos_seed: None,
            },
        ];
        for config in configs {
            let vars = config.to_vars();
            let session = Session::from_vars(&lookup(&vars)).unwrap();
            assert_eq!(*session.config(), config, "{vars:?}");
        }
    }

    #[test]
    fn a_malformed_environment_is_an_error_not_a_default() {
        let good =
            [(ENV_SCHEMA, "1"), (ENV_WORKERS, "4"), (ENV_LAYOUT, "rows:7:9"), (ENV_CHAOS, "off")];
        assert!(session(&good).is_ok());
        for (name, value) in [
            (ENV_SCHEMA, "2"),
            (ENV_WORKERS, "0"),
            (ENV_WORKERS, "many"),
            (ENV_LAYOUT, "rows:0:1"),
            (ENV_LAYOUT, "rows:7"),
            (ENV_LAYOUT, "cols:7:1"),
            (ENV_LAYOUT, "rows:7:1:1"),
            (ENV_CHAOS, "on"),
            (ENV_CHAOS, "-1"),
        ] {
            let mut vars = good;
            vars.iter_mut().find(|(n, _)| *n == name).unwrap().1 = value;
            assert!(session(&vars).is_err(), "{name}={value}");
        }
        for missing in [ENV_WORKERS, ENV_LAYOUT, ENV_CHAOS] {
            let vars: Vec<_> = good.iter().copied().filter(|(n, _)| *n != missing).collect();
            let message = session(&vars).unwrap_err().to_string();
            assert!(message.contains(missing), "{message}");
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    fn records_round_trip_through_a_file_under_the_configuration_the_test_applied() {
        let dir = scratch("records");
        let path = dir.join("hashes.tsv");
        let session = session(&[
            (ENV_SCHEMA, "1"),
            (ENV_WORKERS, "default"),
            (ENV_LAYOUT, "production"),
            (ENV_CHAOS, "7"),
            (ENV_HASH_FILE, path.to_str().unwrap()),
        ])
        .unwrap();
        assert_eq!(session.config().chaos_seed, Some(7));

        // The applied configuration is what the caller passes, not what the session holds.
        let applied = RunConfig {
            workers: NonZeroU32::new(2),
            layout: Layout::Perturbed { capacity: Capacity::Rows(7), seed: 9 },
            chaos_seed: None,
        };
        let mut first = session.recorder("game_sim::falling", &applied).unwrap();
        let mut second = session.recorder("game_sim::rolling", session.config()).unwrap();
        first.record(0, 0xdead_beef).unwrap();
        second.record(0, u64::MAX).unwrap();
        first.record(1, 0).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "applied\tgame_sim::falling\t2\trows:7:9\toff\n\
             applied\tgame_sim::rolling\tdefault\tproduction\t7\n\
             hash\tgame_sim::falling\t0\t00000000deadbeef\n\
             hash\tgame_sim::rolling\t0\tffffffffffffffff\n\
             hash\tgame_sim::falling\t1\t0000000000000000\n"
        );
        let lines = parse_lines(&format!("# a comment\n\n{text}")).unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[0],
            Line::Applied { test: "game_sim::falling".to_owned(), config: applied }
        );
        assert_eq!(
            lines[3],
            Line::Hash(Record { test: "game_sim::rolling".to_owned(), tick: 0, hash: u64::MAX })
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_records_and_names_are_rejected() {
        for text in [
            // Version 1 records, which carried no line kind.
            "t\t0\t0000000000000000",
            "hash\tt\t0",
            "hash\tt\t0\tbeef",
            "hash\tt\tx\t0000000000000000",
            "hash\tt\t+1\t0000000000000000",
            "hash\tt\t0\t000000000000000g",
            "hash\tt\t0\t+000000000000000",
            "hash\t\t0\t0000000000000000",
            "hash\tt\t0\t0000000000000000\textra",
            "applied\tt\t4\tproduction",
            "applied\tt\t0\tproduction\toff",
            "applied\tt\t4\trows:7\toff",
            "launched\tt\t4\tproduction\toff",
        ] {
            let message = parse_lines(&format!("# header\n{text}\n")).unwrap_err().to_string();
            assert!(message.starts_with("line 2 "), "{message}");
        }
        for name in ["", "a\tb", "a\nb", "#a"] {
            assert!(Session::default().recorder(name, &RunConfig::default()).is_err(), "{name:?}");
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the runner passes the mode in the environment and collects hashes in a file (architecture section 8.3)"
    )]
    fn execution_logs_land_at_a_path_that_no_other_test_name_shares() {
        // Review item 7: `a::b` and `a__b` collided under the first mapping.
        let dir = scratch("logs");
        let names = ["a::b", "a__b", "a_3a_3ab", "a b", "a.b", "a-b", "a/b", "é", "A::b", "A::B"];
        // Compared without regard to case, as macOS and Windows compare file names.
        let paths: std::collections::BTreeSet<String> = names
            .iter()
            .map(|name| execution_log_path(&dir, name).to_string_lossy().to_lowercase())
            .collect();
        assert_eq!(paths.len(), names.len());
        assert_eq!(execution_log_path(&dir, "a::b"), dir.join("a_3a_3ab.execlog"));
        assert_eq!(execution_log_path(&dir, "a_3a_3ab"), dir.join("a_5f3a_5f3ab.execlog"));

        let logs = dir.join("not-yet-created");
        let session = session(&[
            (ENV_SCHEMA, "1"),
            (ENV_WORKERS, "4"),
            (ENV_LAYOUT, "production"),
            (ENV_CHAOS, "1"),
            (ENV_LOG_DIR, logs.to_str().unwrap()),
        ])
        .unwrap();
        session.write_execution_log("sim::t", b"schedule").unwrap();
        let path = session.execution_log_path("sim::t").unwrap();
        assert_eq!(path, logs.join("sim_3a_3at.execlog"));
        assert_eq!(fs::read(path).unwrap(), b"schedule");
        fs::remove_dir_all(&dir).unwrap();
    }
}
