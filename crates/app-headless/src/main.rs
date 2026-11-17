//! The headless binary (architecture section 2, "app-headless"): the executor, the IO pool
//! and one server world built from the game's registrations, run for a bounded number of
//! ticks at tick rate from a spawned timer root, the hash printed, the world optionally
//! saved, everything shut down in order. `main` is `crates/app/tests/headless.rs`, section
//! 3, over `game-sim`'s sketch game and the cooked content it builds in memory; the game
//! replaces the sketch at milestone 5 and the asset store replaces the content at milestone
//! 3. The binary declares nothing a world registers: the sketch's component, system and
//! session applier are `game-sim`'s, and the binary holds only the registration values a
//! game crate may not name (D152).
//!
//! Usage: `lactose-app-headless [TICKS] [SLOT]`: `TICKS` (600 by default) ticks, then a save
//! to the file `SLOT` if one is named, its directory created at start. Exits 1 with the
//! error when the policy stops the run, 2 after the report when the save failed.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use lactose_app::{
    Cooked, CrateRegistration, Game, Headless, HeadlessConfig, RunReport, Start, Workers,
    WorldConfig,
};
use lactose_asset_api::{
    Asset, AssetId, AssetKind, AssetSet, AssetVersion, LoadPriority, PinnedTable, PublishedAssets,
    Readiness, UntypedHandle, VersionTable,
};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, PlayerId, Result, Tick};
use lactose_ecs::{RegionId, RegionMode};
use lactose_sim::{RegionBundle, SessionEvent, TickInput};

// ---- The game: `game-sim`'s sketch ----

/// `game-sim`'s registration. A game crate may name neither `lactose-app` nor
/// `lactose-persist`, so the value is the binary's; it passes no setup and no migration
/// function, which a binary may not declare (architecture section 8.1).
static SKETCH_SIM: CrateRegistration = CrateRegistration {
    name: "game-sim",
    types: game_sim::register_types,
    setup: None,
    systems: game_sim::register_systems,
    migrations: None,
};

static GAME: Game =
    Game { name: "sketch", crates: &[SKETCH_SIM], order: None, sessions: Some(game_sim::sessions) };

// ---- The cooked content, in memory ----

/// A store that has published every asset at every version.
struct Store;

struct Blob;

impl Asset for Blob {
    const KIND: AssetKind = AssetKind::from_name("sketch::blob");
    const FORMAT_VERSION: u32 = 1;
}

impl PublishedAssets for Store {
    fn request(&self, _: AssetId, _: AssetVersion, _: LoadPriority) {}

    fn get(&self, id: AssetId, version: AssetVersion) -> Readiness<UntypedHandle> {
        Readiness::Ready(UntypedHandle::new(id, version, Blob))
    }
}

/// The version table of `region`'s one asset.
fn table_of(region: RegionId) -> VersionTable {
    [(AssetId::from_bits(region.0), AssetVersion::from_bits(1).expect("non-zero"))]
        .into_iter()
        .collect()
}

/// One region's cooked content: an empty baseline and one asset.
struct InMemory {
    region: RegionId,
    baseline: Arc<Baseline>,
}

impl InMemory {
    fn new(region: RegionId) -> Result<Self> {
        let baseline = Arc::new(Baseline::from_rows(AssetSet::new(), [], [])?);
        Ok(Self { region, baseline })
    }

    /// The region's bundle in its cooked state, for the first frame.
    fn fresh(&self) -> Result<RegionBundle> {
        Ok(RegionBundle {
            seed: self.region.0,
            mode: RegionMode::Simulated,
            baseline: Arc::clone(&self.baseline),
            overlay: None,
            resources: Vec::new(),
            transient_resources: Vec::new(),
            assets: Some(self.pin(&table_of(self.region))?),
        })
    }
}

impl Cooked for InMemory {
    fn baseline(&self, content_hash: u64) -> Result<Arc<Baseline>> {
        if self.baseline.content_hash() == content_hash {
            Ok(Arc::clone(&self.baseline))
        } else {
            Err(Error::new(ErrorKind::NotFound, "no cooked baseline has that hash"))
        }
    }

    fn table(&self, region: RegionId) -> Option<VersionTable> {
        (region == self.region).then(|| table_of(region))
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        PinnedTable::pin(&Store, table)
            .ready()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "the store has not published it"))
    }
}

/// The frames the run feeds: the region's bundle and player 1's join into it in the first,
/// nothing after.
fn frames(cooked: Arc<InMemory>) -> Result<Box<dyn FnMut(Tick) -> TickInput + Send>> {
    let mut first = Some(cooked.fresh()?);
    let region = cooked.region;
    Ok(Box::new(move |_| {
        let mut input = TickInput::default();
        if let Some(bundle) = first.take() {
            input.loaded.insert(region, bundle);
            input.sessions.insert(PlayerId(1), SessionEvent::Join { region });
        }
        input
    }))
}

// ---- `main` ----

/// What the command line asks for.
struct Args {
    ticks: u32,
    save_to: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self> {
        let usage =
            || Error::new(ErrorKind::InvalidData, "usage: lactose-app-headless [TICKS] [SLOT]");
        let mut args = std::env::args_os().skip(1);
        let ticks = match args.next() {
            Some(ticks) => ticks.to_str().and_then(|ticks| ticks.parse().ok()).ok_or_else(usage)?,
            None => 600,
        };
        let save_to = args.next().map(PathBuf::from);
        if args.next().is_some() {
            return Err(usage());
        }
        Ok(Self { ticks, save_to })
    }
}

/// Architecture section 5.1: a panic writes a crash dump and aborts. At milestone 1 the
/// dump is a text report on stderr (the thread, the panic and a forced backtrace); a
/// minidump and a dump file are `lactose-platform`'s, which does not exist yet.
fn install_crash_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let mut stderr = std::io::stderr().lock();
        // Nothing is left to report a failed write to: the process aborts either way.
        let _ = writeln!(
            stderr,
            "lactose-app-headless crashed\nthread: {}\n{info}\nbacktrace:\n{backtrace}",
            thread.name().unwrap_or("<unnamed>"),
        );
        let _ = stderr.flush();
        std::process::abort();
    }));
}

/// Architecture section 3: every event at `info` and above, the assembly's at target `app`
/// and the policy's decisions among them, as text on stderr; stdout keeps the report.
fn install_subscriber() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing_subscriber::filter::LevelFilter::INFO)
        .init();
}

/// The run: the executor's defaults, the headless world, `ticks` ticks at tick rate from a
/// spawned timer root, the report on stdout, an optional save, an orderly shutdown. The
/// first error is returned, which is the policy's `Stop`; a failed save is logged, as the
/// policy says, the run still reports and shuts down, and the exit status is 2, so a
/// script sees it.
fn main() -> Result<ExitCode> {
    install_crash_hook();
    install_subscriber();
    let Args { ticks, save_to } = Args::parse()?;
    let cooked = Arc::new(InMemory::new(RegionId(1))?);
    let config = HeadlessConfig {
        workers: Workers::default(),
        world: WorldConfig::headless(),
        start: Start::Fresh,
        saves: save_to.as_deref().and_then(Path::parent).map(Path::to_path_buf),
    };
    let mut app = Headless::start(&GAME, config, Arc::clone(&cooked) as Arc<dyn Cooked>)?;
    let report: RunReport = app.run(ticks, frames(cooked)?)?;
    println!(
        "{} ticks in {} frames ({} behind), hash {:#018x}",
        report.ticks, report.frames, report.behind, report.hash
    );
    let mut exit = ExitCode::SUCCESS;
    if let Some(slot) = save_to {
        match app.save(slot.clone()) {
            Ok(()) => println!("saved to {}", slot.display()),
            // `Headless::save` logged it as the policy decides: `Log`, and the run reports.
            Err(error) => {
                eprintln!("the save failed: {error}");
                exit = ExitCode::from(2);
            }
        }
    }
    let _log = app.shutdown();
    Ok(exit)
}
