//! The contract between the SOUR mode runner and a simulation test (architecture section
//! 8.3), version 1.
//!
//! This module depends on nothing but `std` and on nothing else in this crate, so that it can
//! move unchanged into the library the SOUR test crate will depend on; `README.md` records
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
//! | `LACTOSE_SOUR_HASH_FILE` | a path, optional | Where [`Recorder`] appends hash records |
//! | `LACTOSE_SOUR_LOG_DIR` | a path, optional | The directory a chaos run writes its execution log into |
//!
//! With the schema variable set, the three configuration variables are required and a value
//! that does not parse is an error. A test must fail on that error and never fall back to
//! the production configuration, or a mode would pass without having run.
//!
//! # Hash records
//!
//! At every sync point it hashes, a test calls [`Recorder::record`] with the tick and the
//! world hash. Each record is one line, `test<TAB>tick<TAB>hash`, with the hash as sixteen
//! lowercase hexadecimal digits. A line that starts with `#` is a comment. The file is opened
//! for appending and each record is a single write, because nextest runs every test in its
//! own process and they all append to the one file the runner named.
//!
//! # Execution logs
//!
//! A chaos run that fails writes the bytes of its `lactose_task::ExecutionLog` to
//! [`Session::execution_log_path`], so that CI keeps the log as an artifact and the failure
//! replays exactly.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

/// The version of the environment schema and of the record format.
pub const SCHEMA_VERSION: u32 = 1;

/// Names the schema version; its presence means the test runs under the runner.
pub const ENV_SCHEMA: &str = "LACTOSE_SOUR_SCHEMA";
/// Carries [`RunConfig::workers`].
pub const ENV_WORKERS: &str = "LACTOSE_SOUR_WORKERS";
/// Carries [`RunConfig::layout`].
pub const ENV_LAYOUT: &str = "LACTOSE_SOUR_LAYOUT";
/// Carries [`RunConfig::chaos_seed`].
pub const ENV_CHAOS: &str = "LACTOSE_SOUR_CHAOS";
/// Names the file hash records are appended to.
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

fn error<T>(message: String) -> Result<T, ProtocolError> {
    Err(ProtocolError(message))
}

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
    /// The environment variables that carry this configuration, schema version included.
    #[must_use]
    pub fn to_vars(&self) -> Vec<(&'static str, String)> {
        let workers = self.workers.map_or_else(|| "default".to_owned(), |n| n.to_string());
        let layout = match self.layout {
            Layout::Production => "production".to_owned(),
            Layout::Perturbed { capacity, seed } => match capacity {
                Capacity::Rows(n) => format!("rows:{n}:{seed}"),
                Capacity::Bytes(n) => format!("bytes:{n}:{seed}"),
            },
        };
        let chaos = self.chaos_seed.map_or_else(|| "off".to_owned(), |seed| seed.to_string());
        vec![
            (ENV_SCHEMA, SCHEMA_VERSION.to_string()),
            (ENV_WORKERS, workers),
            (ENV_LAYOUT, layout),
            (ENV_CHAOS, chaos),
        ]
    }

    fn from_vars(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, ProtocolError> {
        let required = |name: &str| {
            lookup(name)
                .ok_or_else(|| ProtocolError(format!("{ENV_SCHEMA} is set but {name} is not")))
        };
        let bad = |name: &str, value: &str| -> ProtocolError {
            ProtocolError(format!("{name} has the unrecognised value `{value}`"))
        };

        let value = required(ENV_WORKERS)?;
        let workers = match value.as_str() {
            "default" => None,
            count => Some(count.parse().map_err(|_| bad(ENV_WORKERS, &value))?),
        };

        let value = required(ENV_LAYOUT)?;
        let layout = if value == "production" {
            Layout::Production
        } else {
            let fields: Vec<&str> = value.split(':').collect();
            let [unit, amount, seed] = fields[..] else {
                return Err(bad(ENV_LAYOUT, &value));
            };
            let amount: u32 = amount.parse().map_err(|_| bad(ENV_LAYOUT, &value))?;
            let capacity = match unit {
                "rows" if amount > 0 => Capacity::Rows(amount),
                "bytes" if amount > 0 => Capacity::Bytes(amount),
                _ => return Err(bad(ENV_LAYOUT, &value)),
            };
            Layout::Perturbed { capacity, seed: seed.parse().map_err(|_| bad(ENV_LAYOUT, &value))? }
        };

        let value = required(ENV_CHAOS)?;
        let chaos_seed = match value.as_str() {
            "off" => None,
            seed => Some(seed.parse().map_err(|_| bad(ENV_CHAOS, &value))?),
        };

        Ok(Self { workers, layout, chaos_seed })
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
            return error(format!(
                "{ENV_SCHEMA} is `{schema}` but this test speaks version {SCHEMA_VERSION}; \
                 the runner and the test were built from different revisions"
            ));
        }
        Ok(Self {
            config: RunConfig::from_vars(lookup)?,
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
    /// # Errors
    /// If the name contains a tab or a line break, or the hash file cannot be opened.
    pub fn recorder(&self, test: &str) -> Result<Recorder, ProtocolError> {
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
        Ok(Recorder { test: test.to_owned(), file })
    }

    /// Where the test called `test` writes its execution log if a chaos run fails, or `None`
    /// when the runner keeps no logs. The parent directory may not exist yet.
    #[must_use]
    pub fn execution_log_path(&self, test: &str) -> Option<PathBuf> {
        self.log_dir.as_deref().map(|dir| execution_log_path(dir, test))
    }
}

/// The path of a test's execution log inside a log directory: the test name with every
/// character outside `[A-Za-z0-9_.-]` replaced by `_`, and the extension `.execlog`.
#[must_use]
pub fn execution_log_path(log_dir: &Path, test: &str) -> PathBuf {
    let name: String = test
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' })
        .collect();
    log_dir.join(format!("{name}.execlog"))
}

fn check_test_name(test: &str) -> Result<(), ProtocolError> {
    if test.is_empty() || test.starts_with('#') || test.contains(['\t', '\n', '\r']) {
        return error(format!("`{}` cannot name a test in a hash record", test.escape_debug()));
    }
    Ok(())
}

/// Appends one test's hash records to the file the runner named, or discards them outside
/// the runner.
#[derive(Debug)]
pub struct Recorder {
    test: String,
    file: Option<File>,
}

impl Recorder {
    /// Records the world hash at `tick`.
    ///
    /// # Errors
    /// If the record cannot be written.
    pub fn record(&mut self, tick: u32, hash: u64) -> Result<(), ProtocolError> {
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        let record = Record { test: self.test.clone(), tick, hash };
        // One write per record: appends from concurrent test processes must not interleave.
        file.write_all(format!("{record}\n").as_bytes())
            .map_err(|e| ProtocolError(format!("cannot append a hash record: {e}")))
    }
}

/// One line of a hash file.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Record {
    /// The test that produced the hash.
    pub test: String,
    /// The simulation tick the hash was taken at.
    pub tick: u32,
    /// The world hash.
    pub hash: u64,
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\t{}\t{:016x}", self.test, self.tick, self.hash)
    }
}

/// Parses the text of a hash file. Blank lines and `#` comments are skipped.
///
/// # Errors
/// If a line is not a record. The message names the line.
pub fn parse_records(text: &str) -> Result<Vec<Record>, ProtocolError> {
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let record = match fields[..] {
            // `parse` and `from_str_radix` accept a leading sign; the format does not.
            [test, tick, hash]
                if hash.len() == 16
                    && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                    && tick.bytes().all(|b| b.is_ascii_digit())
                    && check_test_name(test).is_ok() =>
            {
                tick.parse()
                    .ok()
                    .zip(u64::from_str_radix(hash, 16).ok())
                    .map(|(tick, hash)| Record { test: test.to_owned(), tick, hash })
            }
            _ => None,
        };
        match record {
            Some(record) => records.push(record),
            None => return error(format!("line {} is not a hash record: `{line}`", index + 1)),
        }
    }
    Ok(records)
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

    #[test]
    fn outside_the_runner_a_test_gets_the_production_configuration() {
        // The other variables are ignored without the schema variable.
        let session = session(&[(ENV_WORKERS, "3"), (ENV_HASH_FILE, "x")]).unwrap();
        assert_eq!(session, Session::default());
        assert_eq!(*session.config(), RunConfig::default());
        assert_eq!(session.execution_log_path("t"), None);
        let mut recorder = session.recorder("t").unwrap();
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
    fn records_round_trip_through_a_file() {
        let dir =
            std::env::temp_dir().join(format!("lactose-sour-protocol-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hashes.tsv");
        let _ = std::fs::remove_file(&path);
        let path_text = path.to_str().unwrap();
        let session = session(&[
            (ENV_SCHEMA, "1"),
            (ENV_WORKERS, "default"),
            (ENV_LAYOUT, "production"),
            (ENV_CHAOS, "7"),
            (ENV_HASH_FILE, path_text),
            (ENV_LOG_DIR, "logs"),
        ])
        .unwrap();
        assert_eq!(session.config().chaos_seed, Some(7));

        let mut first = session.recorder("game_sim::falling").unwrap();
        let mut second = session.recorder("game_sim::rolling").unwrap();
        first.record(0, 0xdead_beef).unwrap();
        second.record(0, u64::MAX).unwrap();
        first.record(1, 0).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "game_sim::falling\t0\t00000000deadbeef\n\
             game_sim::rolling\t0\tffffffffffffffff\n\
             game_sim::falling\t1\t0000000000000000\n"
        );
        let records = parse_records(&format!("# a comment\n\n{text}")).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[1],
            Record { test: "game_sim::rolling".to_owned(), tick: 0, hash: u64::MAX }
        );
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            session.execution_log_path("game_sim::falling<f32>"),
            Some(Path::new("logs").join("game_sim__falling_f32_.execlog"))
        );
    }

    #[test]
    fn malformed_records_and_names_are_rejected() {
        for text in [
            "t\t0",
            "t\t0\tbeef",
            "t\tx\t0000000000000000",
            "t\t0\t000000000000000g",
            "\t0\t0000000000000000",
            "t\t0\t0000000000000000\textra",
        ] {
            let message = parse_records(&format!("# header\n{text}\n")).unwrap_err().to_string();
            assert!(message.starts_with("line 2 "), "{message}");
        }
        for name in ["", "a\tb", "a\nb", "#a"] {
            assert!(Session::default().recorder(name).is_err(), "{name:?}");
        }
    }
}
