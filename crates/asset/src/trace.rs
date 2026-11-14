//! What the store tells a `tracing` subscriber, recorded under its lock and told once the
//! lock is released: a subscriber's callbacks are code that is not the store's, and none of
//! them may run inside a critical section (`README.md`, "Lookups").
//!
//! A load's span is made lazily for the same reason: creating, cloning and dropping a
//! `Span` each call the subscriber, so the store's map holds a [`LoadTrace`], which is
//! shared without calling anything, and the span inside it is made by the first caller
//! after a lock, whichever thread that is.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use lactose_asset_api::{AssetFailure, AssetId, AssetKind, AssetVersion, LoadPriority};
use tracing::Span;
use tracing::field::{self, Empty, debug};

use crate::LOG_TARGET;
use crate::state::Hex;

/// The `silo.load` span of one load, made on first use.
pub(crate) struct LoadTrace {
    id: AssetId,
    version: Option<AssetVersion>,
    kind: Option<AssetKind>,
    priority: LoadPriority,
    cause: &'static str,
    span: OnceLock<Span>,
}

impl LoadTrace {
    pub(crate) fn new(
        id: AssetId,
        version: Option<AssetVersion>,
        kind: Option<AssetKind>,
        priority: LoadPriority,
        cause: &'static str,
    ) -> Arc<Self> {
        Arc::new(Self { id, version, kind, priority, cause, span: OnceLock::new() })
    }

    /// The span, made now if no caller has made it yet. Never called under the lock.
    pub(crate) fn span(&self) -> &Span {
        self.span.get_or_init(|| {
            tracing::info_span!(
                target: LOG_TARGET,
                parent: None,
                "silo.load",
                asset = %Hex(self.id.to_bits()),
                version = self.version.map(|version| field::display(Hex(version.to_bits()))),
                kind = self.kind.map(|kind| field::display(Hex(kind.0))),
                priority = self.priority.0,
                cause = self.cause,
                bytes = Empty,
                error = Empty,
            )
        })
    }
}

/// One thing to tell the subscriber, in the order a step recorded them.
pub(crate) enum Trace {
    /// A load was accepted, or ended inside its request: its span is made.
    Open(Arc<LoadTrace>),
    /// `silo.current`: an already published version became current.
    Current { id: AssetId, version: AssetVersion, generation: u64 },
    /// `silo.publish`, inside the load's span.
    Publish {
        load: Arc<LoadTrace>,
        id: AssetId,
        version: AssetVersion,
        current: bool,
        generation: u64,
    },
    /// The one `warn` of a recorded failure, inside its load's span, which records the kind.
    Failure {
        load: Arc<LoadTrace>,
        failure: AssetFailure,
        dependency: Option<AssetId>,
        message: Cow<'static, str>,
    },
    /// `silo.search`: what one step's cycle searches cost.
    Search { searches: u64, visits: u64 },
}

impl Trace {
    pub(crate) fn emit(self) {
        match self {
            Self::Open(load) => {
                load.span();
            }
            Self::Current { id, version, generation } => tracing::info!(
                name: "silo.current",
                target: LOG_TARGET,
                asset = %Hex(id.to_bits()),
                version = %Hex(version.to_bits()),
                generation,
            ),
            Self::Publish { load, id, version, current, generation } => tracing::info!(
                name: "silo.publish",
                target: LOG_TARGET,
                parent: load.span(),
                asset = %Hex(id.to_bits()),
                version = %Hex(version.to_bits()),
                current,
                generation,
            ),
            Self::Failure { load, failure, dependency, message } => {
                let span = load.span();
                span.record("error", debug(failure.kind));
                tracing::warn!(
                    target: LOG_TARGET,
                    parent: span,
                    asset = %Hex(failure.id.to_bits()),
                    version = failure.version.map(|version| field::display(Hex(version.to_bits()))),
                    kind = ?failure.kind,
                    dependency = dependency.map(|dependency| field::display(Hex(dependency.to_bits()))),
                    "{message}",
                );
            }
            Self::Search { searches, visits } => tracing::trace!(
                name: "silo.search",
                target: LOG_TARGET,
                searches,
                visits,
            ),
        }
    }
}
