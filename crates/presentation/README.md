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
| `Presentation`, `Presentation::{new, frame, remote}` | The schedule of one shown world: what the remote sample keeps between frames. It runs once a frame on the pair the frame acquired, and lends the remote rows it sampled |
| `PresentView`, `RemoteView` | What a frame is shown at: the frame's fraction, the row the camera follows, and the interpolation buffer and server time the remote rows are sampled at |
| `ScheduleOutput` | What `frame` returns by value: the camera, and the count of remote rows left out |
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
| `lactose-app` (the client's frame, `frame-impl`), as `frame-design` proposes its facade (D162, not merged yet): the render side's `present` and `draw` | In `present`, on the pair the frame acquired: the frame's events delivered, the camera's pivot and boom for the latch, the remote rows sampled and kept for `draw`; in `draw`, the remote rows uploaded with no copy | `Presentation::new()`; `Presentation::frame(&pair, &view, sink)` with the pair `SnapshotReader::acquire` gave and a `PresentView` at the frame's fraction; `ScheduleOutput::{camera, missing}`; `CameraPose::{pivot, boom}` into `CameraLatch`; `upload(ctx, presentation.remote(), \|rows\| writer.write(rows))` with the writer of `Renderer::remote` |
| Audio, effects and the debug overlay (milestone 5) | Requests to start, cancellations to stop, each once | `EventSink::{emitted, cancelled}` |
| Boom collision (milestone 3) | The direction the rig casts along | `Orbit::forward` |
| `game-present`, lights and decals of a world that simulates | A presented row's placement at the frame's time | `interpolate` on the acquired pair |

## Behaviour the signatures cannot express

**The schedule**

- One `Presentation` per shown world. It holds no reader: the frame keeps the world's
  `SnapshotReader`, acquires the pair in its own task and hands it to `frame` inside the
  frame's job (the milestone 2 plan, `frame-design`). A client that shows its prediction
  world and its replicated world holds two. The crate declares nothing a world registers: no
  system, component, resource, applier or hand-written `impl Reflect` (decision 5.18). It
  reads the pair, the interpolation buffer and what the caller passes, and writes nothing a
  world holds.
- `frame` delivers the pair's events, places the camera and samples the remote rows, in that
  order, and returns the camera and the count of remote rows left out. The remote rows stay
  in the `Presentation` until the next call, and `remote` lends them: the frame samples them in
  `present` and uploads them in `draw` with no copy.

**Events**

- `frame` hands every emission of the acquired batch to `EventSink::emitted`, then every
  cancellation to `EventSink::cancelled`, in the batch's order, from the calling thread.
- The events delivered are those the pair carries: the batch of every event published since
  the previous acquire, folded (present-api, "Events and publication"). So each event
  published is delivered once, whatever the cadence of publications and frames, where the
  caller hands every pair it acquires to one call of `frame`, once, as `frame-design`'s frame
  does. A pair handed over twice delivers its events twice; a pair acquired and not handed
  over drops them. A rollback's re-emission of a delivered event is not delivered again; a
  delivered event whose re-simulated tick no longer emits it is cancelled once. Both rest on
  the world's event keys and the channel's fold; this crate keeps no copy of an event.

**Interpolation**

- `interpolate(frame, key, fraction)` is `None` where `PresentFrame::transform` is: no such
  row, or a `Client` world's snapshot, which holds no transform.
- `fraction` is the renderer's `FrameView::fraction`: from zero, the older snapshot's
  transform, to one, the newer's, whatever the ticks between the two snapshots. It is clamped
  to `[0, 1]`, and NaN counts as one. It is the weight, over the whole pair: the renderer draws an instance at `previous + (current - previous) *
  fraction`, its previous and current placements those of the same two snapshots where every
  pass is published and every publication acquired (present-api, "What the sink keeps"). So
  a presented row and the instance the renderer draws for it are shown at the same instant,
  apart from the two exceptions that section names: the camera's pivot is where the mesh it
  follows is drawn, over a pair one tick apart or several.
- A pair `PresentFrame::transform` flags `no_interpolate` (a cut, a row the older snapshot does
  not hold, a pair of one tick, a `Teleported` after the older snapshot's tick) shows the newer
  transform at every fraction. A body despawned and spawned again under its key between the
  two snapshots, with no `Teleported`, is in both and is interpolated from where it was.
- Otherwise the translation is `lerp` and the rotation `slerp` of `lactose_math`, at the
  fraction; a fraction of zero or one gives the older or the newer transform exactly.

**The camera rig** (architecture section 13)

- With a focus whose row the newer snapshot holds with a `Transform` and a `CameraRig`,
  `ScheduleOutput::camera` is `CameraPose { pivot, boom }`: the pivot
  `RenderPosition::from_sim` of `CameraRig::pivot_of` the row's interpolated transform, by the
  newer snapshot's rig; the boom its rest length as an `f32`, zero where it is negative or NaN.
  Without one it is `None`, and the frame keeps whatever camera it had.
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
  each within the sum of the two angles' 32-bit rounding, below 2.5e-7. That holds the rig's
  statement, not the drawn camera's: the camera drawn is `CameraData::orbit`'s, which
  `lactose-renderer` holds to `aim` and `Yaw::rotation` itself (verification row 95), since it
  cannot depend on this crate.

**Remote rows** (architecture section 5.3)

- With a `RemoteView`, `frame` takes every row of the newer snapshot that has a `RemoteSlot`
  and a `MeshInstance`, and samples their slots in one `InterpolationReader::sample_batch`
  call at `RemoteView::at`, each slot once. A call on the same pair samples the same rows
  again at the new time.
- A remote row's placement exists only as its row in `remote`, so only a remote row with a
  mesh has one. A remote row without a `MeshInstance` is not sampled and gets no placement:
  a remote entity's light, decal, marker or `CameraRig` (a spectator's camera) is not placed
  by this crate, and `interpolate` is `None` on a `Client` world's pair, which holds no
  transform. Sampling those is a later milestone's; milestone 2 shows no remote world with
  lights.
- A sampled row's placement is `RenderTransform::from_sim` of the sample's position and
  rotation, `Interpolated` or `Held`. A row whose slot samples `Missing` (no state, or
  released and perhaps reused) keeps the placement it had in the previous frame's rows, as
  net-host asks; one with none is left out of the rows and counted in
  `ScheduleOutput::missing`.
- `remote` is ascending by key, each key once, so that each remote instance is written
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
  written from the calling job; more are written from forked child jobs of `ctx`, at once, in
  no stated order and on no stated thread (the calling job's among them).
- `write` must not need a lock or anything else the calling job holds across `upload`: a
  child may run on the calling job's thread, which does not return before every child has. A
  panic in `write` aborts the process, as any job's does (architecture section 5.1).
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
| The camera rig's orbit (verification row 95, this crate's part) | `camera.rs`: for the six samples whose derived values `lactose-input` pins, and every 97th yaw unit at every 129th pitch unit with both limits, the raw sample at the sample's angles quantises back to it, and `Orbit::forward` and `Orbit::yaw_rotation` are its `aim` and `Yaw::rotation` within the angles' rounding; the cardinal directions of the world frame; the pose: the pivot is the rig's offset turned with the row from the interpolated transform, the boom the rest length, zero for a negative or NaN one, and no camera without a focus or a rig |
| A re-spawned body (verification row 99) | `interpolation.rs`: a prediction world's body despawned and spawned again under its key at x 10, from x 0: without `Teleported` the frame shows x 5 at fraction 0.5 and the camera's pivot with it; with `Teleported` at the newer snapshot's tick it shows x 10 at every fraction; one teleported at the older snapshot's tick blends. The fraction as the weight over a pair two ticks apart, its exact ends, the clamp and NaN; the rotation `slerp`ed; a pair of one tick shown as it is |
| The camera with the mesh it follows | `interpolation.rs`: a row with a mesh and a `CameraRig` moved 4 m a tick over one, two and five ticks between publications, extract's rows written to a model of the renderer's sink (present-api, "What the sink keeps"): at nine fractions of the frame the camera's pivot is where `FrameView::fraction`'s rule draws the mesh |
| Each remote slot written once a frame | `remote.rs`: a client world of 600 remote rows, a row with a slot and no mesh, one with a mesh and no slot, and one whose slot never received a state: every remote row once, ascending, with its mesh, at its sample, frame after frame with and without a publication; a released slot keeps its row where it was shown, one never shown is left out and counted; a despawned row is gone; a frame with no remote view has none. The upload of 0, 1, 256, 257 and 600 rows: each row in exactly one call, none empty, none over 256, the calls together the rows; and of 600 rows on four workers, each call waiting until two have been in progress at once: two were. That the rows are sampled in one `sample_batch` call, each slot once, is by inspection: no test counts the samples |
| Events delivered once | `events.rs`: a delivered event re-emitted by a rollback is not delivered again, and nothing is published for it; one whose re-simulated tick no longer emits it is cancelled once, in the next frame; a frame with nothing published since delivers nothing. Under four cadences of frames against publications with a two-tick rollback every eighth step: no key emitted or cancelled in two frames, and the net delivered of each key the net published |
| Outside the boundary | `cargo xtask boundary` passes with the crate outside `[lint_file_carriers]`: it declares nothing a world registers. The tests alone declare one system, `test::chirp`, which emits a sound request and writes no component |
| Cost (steering) | `cost.rs`, ignored, run by hand in release: 5,000 remote rows sampled and uploaded to a counting closure at 144 Hz for ten seconds, with a network thread pushing a state into every slot at 30 Hz; the median and the 99th percentile of the frame, and of the batch call made a second time after the frame, are printed. The ring's copy is not in it while `RemoteWriter::write` is the facade's. The median varies between runs, from about 0.26 to 1.8 ms, and the cause is not measured (the test's module comment) |
