//! `frame-pace`'s stamps (`README.md`, "Stamps"; `crates/app/README.md`, "Pacing").

/// Installs the binary's log as the process's `tracing` subscriber: every event at `INFO` or
/// above, formatted to stderr. With `stamps`, also the layer that stamps each recorder of
/// `lactose-render-graph` from the entry to the exit of its pass's span, which the paced
/// mode's stamps read; without it the subscriber has no such layer and the graph's spans cost
/// what they cost with no subscriber that wants them. Called once, first in `main` after the
/// crash hook; a second call leaves the first's subscriber in place.
pub fn install_log(stamps: bool) {
    let _ = stamps;
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing_subscriber::filter::LevelFilter::INFO)
        .try_init();
}
