//! Compile-only sketch of one tick of the milestone 1 netcode spike's loop against this
//! surface (architecture section 22): the server's frame, a join, the relevance message
//! that becomes the client's first full frame, the client's frame and what it sends back.
//! The transport, the packing and the jitter buffer are the spike's own and are stubbed.
//! Type-checked, never run (every deciding body in the facade is `todo!()`). A change that
//! breaks this sketch is a signature change to list.

#![forbid(unsafe_code)]
#![allow(dead_code, unused_variables, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lactose_app::{
    Client, ClientError, Cooked, Game, Server, ServerError, Start, Workers, WorldConfig,
    build_client, build_server, start_executor,
};
use lactose_asset_api::{PinnedTable, VersionTable};
use lactose_bundle::Baseline;
use lactose_core::{Error, ErrorKind, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{RegionId, World};
use lactose_input::{PlayerInput, SampleHistory};
use lactose_net_api::{RegionChange, StagedFrame, StagingFrame};
use lactose_net_host::ClockEstimate;
use lactose_net_host::message::{JoinAccepted, ResyncReason, ResyncRequest};
use lactose_net_host::transport::{ConnectionId, LinkStats, Transport, TransportEvent};
use lactose_sim::{PlayerSample, SessionApplier, SessionEvent, TickInput};
use lactose_task::Executor;

/// The spike's game: the engine's crates and one session applier are enough to walk a
/// player about.
static GAME: Game =
    Game { name: "spike", crates: &[], order: None, sessions: Some(spike_sessions) };

/// The spike's session applier: spawns the predicted character with its claim, anchor,
/// transform and body on `Join`; despawns it on `Leave`.
struct SpikeSessions;

impl SessionApplier for SpikeSessions {
    fn apply(&mut self, world: &mut World, player: PlayerId, event: SessionEvent) -> Result<()> {
        unimplemented!("the spike's player entity")
    }
}

fn spike_sessions() -> Box<dyn SessionApplier> {
    Box::new(SpikeSessions)
}

/// The lead the spike runs its prediction world ahead by, until a clock estimate says.
const LEAD: u32 = 2;

// ---- The cooked content both ends hold in memory ----

struct InMemory {
    region: RegionId,
    baseline: Arc<Baseline>,
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
        (region == self.region).then(VersionTable::default)
    }

    fn pin(&self, table: &VersionTable) -> Result<PinnedTable> {
        unimplemented!("the spike's store pins its one table")
    }
}

// ---- The transport, which is the spike's ----

struct Throwaway;

impl Transport for Throwaway {
    fn send(&mut self, to: ConnectionId, payload: &[u8]) -> Result<()> {
        unimplemented!("the spike's UDP transport")
    }

    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        unimplemented!("the spike's UDP transport")
    }

    fn stats(&self, connection: ConnectionId) -> Option<LinkStats> {
        None
    }

    fn disconnect(&mut self, connection: ConnectionId) {}
}

/// The spike's packing of a gathered frame for its one client; the real one is `net`'s.
fn pack(gathered: &StagingFrame, tick: Tick) -> Arc<StagedFrame> {
    unimplemented!("the spike's packer")
}

fn encode_inputs(history: &SampleHistory) -> Vec<u8> {
    unimplemented!("the spike's input packet")
}

fn encode_resync(request: &ResyncRequest) -> Vec<u8> {
    unimplemented!("the spike's resync packet")
}

// ---- The server's half ----

/// What the spike's server keeps beside the world: the session events that arrived and
/// the inputs its jitter buffer released, which the frame callback drains into each tick.
#[derive(Default)]
struct Inbound {
    sessions: BTreeMap<PlayerId, SessionEvent>,
    inputs: BTreeMap<PlayerId, PlayerSample>,
}

/// Start: the executor, then the server world, fresh.
fn server_start() -> Result<(Executor, Server)> {
    let executor = start_executor(Workers::default())?;
    let server = build_server(&executor, &GAME, WorldConfig::headless(), Start::Fresh)?;
    Ok((executor, server))
}

/// One server frame: the due ticks take what arrived, the passes run, the gather's frame
/// is packed for the client. A server failure is the policy's `Stop`: the error goes up.
async fn server_frame(
    executor: &Executor,
    server: Server,
    inbound: &mut Inbound,
    now: Instant,
) -> Result<(Server, Arc<StagedFrame>)> {
    let mut frames = |tick: Tick| {
        Some(TickInput {
            sessions: std::mem::take(&mut inbound.sessions),
            inputs: std::mem::take(&mut inbound.inputs),
            ..TickInput::default()
        })
    };
    let (server, out) = match server.frame(executor, now, &mut frames).await {
        Ok(done) => done,
        Err(ServerError { error, server: _ }) => return Err(error),
    };
    let packed = pack(&out.gathered, server.tick());
    executor.sleep_until(out.next_deadline).await?;
    Ok((server, packed))
}

/// A join, after the tick that carried its session event ran: the player's entity as the
/// world named it, the acceptance, and the first full frame from the relevance message of
/// the region the player stands in.
fn accept_join(
    server: &Server,
    player: PlayerId,
    region: RegionId,
    cooked: &InMemory,
) -> Result<(JoinAccepted, StableKey, Arc<StagedFrame>)> {
    let entity = lactose_input::player_entity(server.simulation().world(), player)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "the join's tick has not run"))?;
    let accepted = JoinAccepted { player, tick: server.tick() };
    let relevance = server.relevance(region, cooked.table(region))?;
    let mut frame = StagedFrame::resync(server.tick());
    frame.regions.insert(region, RegionChange::Relevant(relevance));
    Ok((accepted, entity, Arc::new(frame)))
}

// ---- The client's half ----

/// On `JoinAccepted`: the prediction world at the join tick plus the lead, with the first
/// full frame queued before its first frame runs.
fn client_join(
    executor: &Executor,
    accepted: JoinAccepted,
    entity: StableKey,
    first: Arc<StagedFrame>,
    cooked: Arc<InMemory>,
) -> Result<Client> {
    let mut client =
        build_client(executor, &GAME, WorldConfig::headless(), accepted, entity, LEAD, cooked)?;
    if !client.push(first) {
        return Err(Error::new(ErrorKind::InvalidData, "the first full frame was refused"));
    }
    Ok(client)
}

/// The spike's scripted input: walk forward.
fn scripted(tick: Tick) -> PlayerInput {
    PlayerInput::IDLE
}

/// One client frame: the staged frame the transport delivered is pushed, the frame runs
/// with the scripted sample, the last eight samples go back as the input packet, a resync
/// becomes a request, and the loop sleeps to the next deadline. A kept failure is the
/// policy's `Resync`: the client is kept and the next frame reports the resync; a torn
/// one is `Stop`.
async fn client_frame(
    executor: &Executor,
    client: Client,
    delivered: Option<Arc<StagedFrame>>,
    now: Instant,
    transport: &mut dyn Transport,
    to: ConnectionId,
) -> Result<Client> {
    let mut client = client;
    if let Some(frame) = delivered
        && !client.push(frame)
    {
        // Refused: an old frame, logged by `push`, entering nothing.
    }
    let mut sample = scripted;
    let (client, out) = match client.frame(executor, now, &mut sample).await {
        Ok(done) => done,
        Err(ClientError { error, client: Some(kept) }) => return Ok(kept),
        Err(ClientError { error, client: None }) => return Err(error),
    };
    // What the milestone measures: the rollback cost per re-simulated tick.
    let per_tick: Option<Duration> =
        (out.reconciled.resimulated > 0).then(|| out.rollback / out.reconciled.resimulated);
    let corrections = out.reconciled.corrections.len();
    transport.send(to, &encode_inputs(client.history()))?;
    if let Some(tick) = out.resync {
        let request = ResyncRequest { tick, reason: ResyncReason::BehindCatchUp };
        transport.send(to, &encode_resync(&request))?;
    }
    executor.sleep_until(out.next_deadline).await?;
    Ok(client)
}

/// The spike's clock estimate from its own round trip, handed to the timer root between
/// frames; `net` computes the real one at milestone 4.
fn clock_from_round_trip(client: &mut Client, now: Instant, server_tick: Tick, rtt: Duration) {
    let half = rtt.as_secs_f32() / 2.0;
    client.clock(
        now,
        ClockEstimate {
            server_tick,
            lead_ticks: half * 60.0 + 1.0,
            rate_thousandths: 1000,
            snap: false,
        },
    );
}

/// One tick of the spike end to end, both ends in one process: the server's frame, its
/// packed frame delivered, the client's frame, its inputs delivered back.
async fn one_tick(
    executor: &Executor,
    server: Server,
    client: Client,
    inbound: &mut Inbound,
    transport: &mut dyn Transport,
    to: ConnectionId,
) -> Result<(Server, Client)> {
    let (server, packed) = server_frame(executor, server, inbound, Instant::now()).await?;
    let client =
        client_frame(executor, client, Some(packed), Instant::now(), transport, to).await?;
    let divergence = server.hash()? != client.hash()?;
    Ok((server, client))
}
