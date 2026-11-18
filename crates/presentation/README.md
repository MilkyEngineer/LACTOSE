# lactose-presentation

The presentation schedule (architecture sections 5.3 and 13): what runs once a render frame on
a world's presentation snapshot. It interpolates between the snapshot pair, places the camera
rig's pivot and boom, samples every remote entity of a client's replicated world from the
interpolation buffer in one batch call and writes the placements through the renderer's
ring, and hands the frame's presentation events to a sink, each once.

| | |
|---|---|
| Tier | backend |
| Determinism boundary | outside; not in `[boundary_internal]`, and no carrier of the boundary's lint file (`[lint_file_carriers]`): it declares nothing a world registers |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented for milestone 2**, as decision 5.5 of `docs/plans/milestone-2.md` scopes
it. Every sentence under "Behaviour the signatures cannot express" is one a dependent crate
may rely on; "Proof for milestone 2" names the test that holds each. Architecture sections
5.3, 12.4 and 13 are the design; where this file and the architecture disagree, the
architecture wins and this file is wrong. `crates/present-api/README.md` (the snapshot, the
triple buffer, the no-interpolate flag, events and publication), `crates/net-host/README.md`
(the interpolation buffer and its sampler) and `crates/renderer/README.md` (the remote batch,
the latch) are the contracts this crate builds on and never restates.

Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `lactose-app` (the client's frame) is written
against this surface.

## Surface

| Item | Purpose |
|---|---|
| `Presentation`, `Presentation::{new, frame}` | The schedule of one shown world: it holds the world's `SnapshotReader` and what the remote sample keeps between frames, and runs once a frame |
| `PresentView`, `RemoteView` | What a frame is shown at: the frame's fraction, the row the camera follows, and the interpolation buffer and server time the remote rows are sampled at |
| `Shown` | What a frame shows: the acquired pair, the camera, the remote rows and the count of those left out |
| `EventSink` | Where the frame's events go, each once: `emitted`, `cancelled` |
| `interpolate` | The transform a frame shows a presented row at |
| `CameraPose` | The pivot and the boom a frame's camera is latched with |
| `Orbit`, `Orbit::{of, forward, yaw_rotation}` | The rig's yaw and pitch from the newest raw sample, and its directions |
| `upload` | The remote rows written from the frame's job, in forked writes |

Hidden plumbing: none. The private fields of `Presentation` are the implementation's to
reshape.

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `lactose-app` (the client's frame, `frame-impl`) | Once a frame, beside the next ticks: the camera's pivot and boom for the latch, the remote rows for the remote batch, the frame's events delivered; the pair for what else the frame reads of the snapshot | `Presentation::new` with the world's `SnapshotReader`; `Presentation::frame` with a `PresentView`; `Shown::{frame, camera, remote, missing}`; `CameraPose::{pivot, boom}` into `CameraLatch`; `upload(ctx, shown.remote, \|rows\| writer.write(rows))` with the writer of `Renderer::remote` |
| Audio, effects and the debug overlay (milestone 5) | Requests to start, cancellations to stop, each once | `EventSink::{emitted, cancelled}` |
| Boom collision (milestone 3) | The direction the rig casts along | `Orbit::forward` |
| `game-present`, lights and decals | A presented row's placement at the frame's time | `interpolate` on `Shown::frame` |

## Behaviour the signatures cannot express

**The schedule**

- One `Presentation` per shown world, holding that world's `SnapshotReader`. A client that
  shows its prediction world and its replicated world holds two. The crate declares nothing a
  world registers: no system, component, resource, applier or hand-written `impl Reflect`
  (decision 5.18). It reads the snapshot, the interpolation buffer and what the caller passes,
  and writes nothing a world holds.
- `frame` acquires once. It returns `None` before the world's first publication, and delivers
  nothing then: the events published so far wait for the first frame (present-api). Otherwise
  it delivers the frame's events, places the camera and samples the remote rows, in that order,
  and returns them with the pair.
- `Shown::frame` is the pair `frame` acquired. Its events are delivered already: a caller
  hands them to nothing else.

**Events**

- `frame` hands every emission of the acquired batch to `EventSink::emitted`, then every
  cancellation to `EventSink::cancelled`, in the batch's order, from the calling thread.
- Each event published is delivered by exactly one call of `frame`, whatever the cadence of
  publications and frames: each call acquires once, and the batch it acquires holds every event
  published since the previous acquire, folded (present-api, "Events and publication"). A
  rollback's re-emission of a delivered event is not delivered again; a delivered event whose
  re-simulated tick no longer emits it is cancelled once. Both rest on the world's event keys
  and the channel's fold; this crate adds no copy that could deliver one twice.

**Interpolation**

- `interpolate(frame, key, fraction)` is `None` where `PresentFrame::transform` is: no such
  row, or a `Client` world's snapshot, which holds no transform.
- `fraction` is the renderer's `FrameView::fraction`: from zero, the tick before the newer
  snapshot's, to one, the newer snapshot's tick. It is clamped to `[0, 1]`, and NaN counts as
  one. The frame's time is that point, and the pair is interpolated linearly over its ticks:
  the weight is `fraction` for snapshots a tick apart and `(1 + fraction) / 2` two ticks
  apart. So a snapshot row and an instance the renderer draws between its two placements at
  the same fraction are shown at the same time.
- A pair `PresentFrame::transform` flags `no_interpolate` (a cut, a row the older snapshot does
  not hold, a pair of one tick, a `Teleported` after the older snapshot's tick) shows the newer
  transform at every fraction. A body despawned and spawned again under its key between the
  two snapshots, with no `Teleported`, is in both and is interpolated from where it was.
- Otherwise the translation is `lerp` and the rotation `slerp` of `lactose_math`, at the
  weight; a weight of zero or one gives the older or the newer transform exactly.

**The camera rig** (architecture section 13)

- With a focus whose row the newer snapshot holds with a `Transform` and a `CameraRig`,
  `Shown::camera` is `CameraPose { pivot, boom }`: the pivot `RenderPosition::from_sim` of
  `CameraRig::pivot_of` the row's interpolated transform, by the newer snapshot's rig; the
  boom its rest length as an `f32`, zero where it is negative or NaN. Without one it is `None`,
  and the frame keeps whatever camera it had.
- The boom is the rest length at milestone 2: boom collision needs the static collision asset
  of milestone 3 (decision 5.5), and `CameraLatch::boom` is the value the collision will
  find.
- The orbit is the latch's: `Renderer::latch` reads the newest raw sample's yaw and pitch
  itself and computes the camera with `CameraData::orbit`, the one statement of the latched
  camera. `Orbit` is the rig's own view of the same angles, in the world frame of section 5.3
  (right-handed, `+Y` up, yaw zero facing `-Z`, positive yaw turning left, positive pitch
  looking up): `forward` is `-Z` turned by the pitch about `+X`, then by the yaw about `+Y`;
  `yaw_rotation` the turn about `+Y` alone; both in `lactose_math`'s scalars from the 32-bit
  angles.
- The rig's input carries its angles into the simulation through `Quantiser::next`, so for a
  raw sample at a quantised sample's angles, `forward` is that sample's `PlayerInput::aim` and
  `yaw_rotation` its `Yaw::rotation` (as a rotation: a quaternion and its negation are one),
  each within the sum of the two angles' 32-bit rounding, below 2.5e-7. The camera looks where
  the simulation aims.

**Remote rows** (architecture section 5.3)

- With a `RemoteView`, `frame` takes every row of the newer snapshot that has a `RemoteSlot`
  and a `MeshInstance`, and samples their slots in one `InterpolationReader::sample_batch`
  call at `RemoteView::at`, each slot once. A row with a slot and no mesh is not drawn and not
  sampled. A call with no publication since the last samples the same rows again at the new
  time.
- A sampled row's placement is `RenderTransform::from_sim` of the sample's position and
  rotation, `Interpolated` or `Held`. A row whose slot samples `Missing` (no state, or
  released and perhaps reused) keeps the placement it had in the previous frame's rows, as
  net-host asks; one with none is left out of the rows and counted in `Shown::missing`.
- `Shown::remote` is ascending by key, each key once, so that each remote instance is written
  once a frame: the renderer counts a second write of one as a duplicate. Every row carries
  `no_interpolate` set: the renderer sets both placements of a remote row to the row's
  whatever the flag says, and nothing is drawn between two samples.
- Every remote instance with a placement is in the rows every frame, so the frame's remote
  batch writes each again after a `Client` full pass freed them (`lactose-renderer`, "Slots").
- A frame with no `RemoteView` has no remote rows and drops the previous frame's placements.
- The sample allocates nothing in a steady frame: its buffers are kept between frames and grow
  only with the number of remote rows.

**The upload**

- `upload(ctx, rows, write)` calls `write` with every row in exactly one call, no call empty
  and none over 256 rows, and returns once every call has returned. Rows that fit one call are
  written from the calling job; more are written from forked child jobs of `ctx`, at once.
- `write` is `RemoteWriter::write` of the writer `Renderer::remote` makes, which may be called
  from several jobs at once and waits for nothing. Making that writer after the frame's
  `Client` pass has ended, and dropping it, which closes the frame's one remote batch, before
  `prepare`, are the frame's to order (`lactose-renderer`, "Slots" and "Concurrency"). The
  library names nothing of the renderer: the writer reaches it as the closure.

## Not in this crate yet

- Boom collision (milestone 3, with the static collision asset); pose evaluation (milestone 3,
  architecture section 22); the visual blend of a correction (milestone 4); the prediction
  world's copy shadowing the replicated one, tested when `Client` holds both worlds (milestone
  4); UI animation, the audio listener and markers (milestone 5). Decision 5.5.
- The presentation time of the remote rows: `RemoteView::at` is the caller's, from the clock
  estimate, about two send intervals behind the newest received tick (net-host).
- Device sources: every proof drives the camera with scripted samples.

## Proof for milestone 2

The tests in `tests/`, one binary (D150), asserting on values, counts and orders and never on
timing.

| What | How |
|---|---|
| The camera rig's orbit (verification row 95) | `camera.rs`: for the six samples whose derived values `lactose-input` pins, and every 97th yaw unit at every 129th pitch unit with both limits, the raw sample at the sample's angles quantises back to it, and `Orbit::forward` and `Orbit::yaw_rotation` are its `aim` and `Yaw::rotation` within the angles' rounding; the cardinal directions of the world frame; the pose: the pivot is the rig's offset turned with the row from the interpolated transform, the boom the rest length, zero for a negative or NaN one, and no camera without a focus or a rig |
| A re-spawned body (verification row 99) | `interpolation.rs`: a prediction world's body despawned and spawned again under its key at x 10, from x 0: without `Teleported` the frame shows x 5 at fraction 0.5 and the camera's pivot with it; with `Teleported` at the newer snapshot's tick it shows x 10 at every fraction; one teleported at the older snapshot's tick blends. The weight over a pair two ticks apart, its exact ends, the clamp and NaN; the rotation `slerp`ed; a pair of one tick shown as it is |
| Each remote slot sampled and written once a frame | `remote.rs`: a client world of 600 remote rows, a row with a slot and no mesh, one with a mesh and no slot, and one whose slot never received a state: every remote row once, ascending, with its mesh, at its sample, frame after frame with and without a publication; a released slot keeps its row where it was shown, one never shown is left out and counted; a despawned row is gone; a frame with no remote view has none. The upload of 0, 1, 256, 257 and 600 rows: each row in exactly one call, none empty, none over 256, the calls together the rows |
| Events delivered once | `events.rs`: a delivered event re-emitted by a rollback is not delivered again, and nothing is published for it; one whose re-simulated tick no longer emits it is cancelled once, in the next frame; a frame with nothing published since delivers nothing. Under four cadences of frames against publications with a two-tick rollback every eighth step: no key emitted or cancelled in two frames, and the net delivered of each key the net published |
| Outside the boundary | `cargo xtask boundary` passes with the crate outside `[lint_file_carriers]`: it declares nothing a world registers. The tests alone declare one system, `test::chirp`, which emits a sound request and writes no component |
| Cost (steering) | `cost.rs`, ignored, run by hand in release: 5,000 remote rows sampled and uploaded to a counting closure at 144 Hz for ten seconds, with a network thread pushing a state into every slot at 30 Hz; the median and the 99th percentile of the frame, and of the batch call alone, are printed. The ring's copy is not in it while `RemoteWriter::write` is the facade's |
