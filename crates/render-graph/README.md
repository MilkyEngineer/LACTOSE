# GOUDA (`lactose-render-graph`)

The render graph (architecture section 12.3): a frame's passes, declared by the images and
buffers they use, compiled into the passes that are kept, the lifetimes and aliased memory of
the transients, the state transitions between passes and the queue each pass runs on; then
recorded on the frame-critical workers and submitted in graph order. It reaches a device
through `lactose-rhi-api` alone and names no backend. `lactose-renderer` and a client binary
name this crate; `lactose-app` never does (milestone 2 plan, decision 5.11).

| | |
|---|---|
| Tier | backend |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: facade.** Every public signature is final for milestone 2's lanes and every body
that decides anything is `todo!()`. What runs is plain data: the ids, a key's value and
`History`. This file is the contract `graph-impl` meets, and every sentence under "Behaviour
the signatures cannot express" is one a dependent crate may rely on. Architecture sections
12.3, 3 and 5.1 are the design, with the milestone 2 plan's decisions 5.4, 5.6, 5.11 and
5.17; where this file and the architecture disagree, the architecture wins and this file is
wrong. Decision D160 records the choices section 12.3 left open. The surface below it is
`crates/rhi-api/README.md` (D158), cited as **rhi-api**, and `crates/task/README.md`. The two
spikes are the evidence for what a device does with each call, cited as **RHI**
(`docs/results/spike-rhi-apple-m1-2026-11-15.md`) and **Metal**
(`docs/results/spike-metal-apple-m1-2026-11-17.md`).

The interface is unstable (D43) until `graph-impl` and `renderer-impl` have been written
against it. Amending a signature is allowed in the change that needs it. Say so in the change
description and list what changed, because `lactose-renderer` and `lactose-app-client` are
written against this surface. `tests/consumers.rs` is a compile-only sketch of what each
consumer writes: a change that breaks one is a signature change to list.

## Surface

| Item | Purpose |
|---|---|
| `RenderGraph`, `GraphDesc`, `GraphStats` | The graph of one device: `new`, `device`, `compile`, `record`, `submit`, `last_frame`, `stats`; what it is made with; its counts |
| `FrameDecl` | One frame's declaration: `new`, `views`, `import_image`, `import_buffer`, `transient_image`, `render_pass`, `compute_pass`, `copy_pass` |
| `ImageImport`, `BufferImport`, `TransientImage` | What an image or a buffer is declared with |
| `ImageId`, `BufferId`, `PassId` | What a declaration hands back: a number in that declaration |
| `RenderPassDecl`, `ComputePassDecl`, `CopyPassDecl`, `ColourUse`, `DepthUse`, `BufferUse`, `BufferAccess` | What a pass uses, and how |
| `RenderRecorder`, `ComputeRecorder`, `CopyRecorder` | What a pass's recorder is lent: the commands of its kind of pass, over ids |
| `CompiledFrame`, `CompiledPlan`, `PlannedPass`, `TransientMemory`, `GraphKey` | What a declaration compiled to: the kept passes in graph order with queue and submit, which passes were culled, where each transient lives, the key, and whether this compile made the plan |
| `RecordedFrame` | A frame recorded and not yet submitted |
| `History` | The history ping-pong: two resources of the caller's that swap roles every frame |
| `LOG_TARGET` | `gouda`, the target of the graph's spans |

The crate exports no trait and no `#[doc(hidden)]` item. Its one piece of private plumbing
is `sync_view::SyncView`, a `Sync` cell reached through `&mut` alone ("Concurrency").

## What each consumer needs, and where this surface gives it

| Consumer | Needs | Given by |
|---|---|---|
| `graph-impl`'s tests on null and on devices (its plan row) | Each proof written against the surface with no hook: which passes were culled; whether a compile was a recompile; that two transients alias; null's refusal by the rule it names; both queue paths; a frame held pending, and the encoders a lagging device costs | `CompiledPlan::is_culled`, `passes`; `CompiledFrame::fresh`, `GraphKey`, `GraphStats`; `CompiledPlan::transient` and `TransientMemory::Placed`; `submit` returns a device's error unchanged, so `Rule::of` names the rule; `PlannedPass::queue` and `submit`, `CompiledPlan::submits`; an import's `ready`, which holds a frame's submit behind a point the test signals; `GraphStats::encoders` |
| `lactose-renderer` (`renderer`, `renderer-impl`; decision 5.4) | The staging ring read and the persistent instance buffer written through addresses by the scatter; instanced triangles, indexed and indirect, into an offscreen target; a camera buffer written after every pass is recorded; a readback in device tests; a pass-provider trait whose passes' recorders borrow the provider | `BufferImport`, `BufferAccess`; `ComputeRecorder::push_address`, `dispatch`; `RenderRecorder::bind_index_buffer`, `draw_indexed_indirect`; `record` and `submit` as two calls with the latch between; `CopyPassDecl` and a `BufferImport` whose `last` is `HostRead`; `FrameDecl<'f>` takes each recorder as `impl FnOnce + Send + 'f` |
| `lactose-terrain` and `lactose-debug-ui`, which `crate-tiers.toml` gives the renderer alone; later `lactose-ui`, which it gives this crate too | To add passes through the renderer's pass-provider trait, the first two without naming this crate | Nothing here for the first two: the renderer's facade re-exports what its pass-provider trait names, and `FrameDecl` and the pass types are plain enough to re-export. `lactose-ui` may name them here or there |
| The client's frame (`frame-design`, `frame-impl`): `lactose-app-client`, and `lactose-app` through a `dyn` object | Compile, record, latch and submit as steps of the frame; objects that wait between steps in a world's resource or a `dyn` object's fields; record from a job, under the physics reservation; the frame's point, for the present thread and for pacing; a target handed to the present role and taken back | `RenderGraph` and `RecordedFrame` are `Send + Sync + 'static` and borrow nothing; `record` takes `&mut JobContext` and forks in its scope; `submit` returns a `TimelinePoint`; `ImageImport::handoff` and `ready`. No trait here is one `lactose-app` must name |
| The present thread (`swapchain`, `present-impl`) | A finished target in a known state, released toward the present role where that is a transfer, and a point to wait for | `ImageImport::last` and `handoff`; the frame's point |

Three things these consumers must know that no signature shows:

- **`frame-design` and the present thread: the present thread never acquires a release it
  skipped.** A target released toward the present role and taken back from `Undefined` by
  a later frame is accepted whether or not the present thread acquired it: null's
  transition from `Undefined` ends a release no acquire met. rhi-api's "States" says so
  since the `rhi-null` lane (D160, and D158's addendum at `rhi-null`), and
  `acquire_of_a_release_a_retake_ended_is_refused` holds it. A present thread that
  then acquires a release older than the frame's re-take is refused on null (`no-qfot`:
  an acquire with no equal release submitted), and on Vulkan races the graphics queue's
  writes.
- **`frame-design` and `frame-impl`: the one lock-free way to hold `&mut RenderGraph` and
  `&mut JobContext` together** is a system with a `Job` parameter, whose `context()` is
  the `&mut JobContext` that `record` takes, beside a `ResMut` of a `dyn` trait. The trait
  belongs to a foundation crate and its method takes `&mut JobContext`;
  `World::insert_service` holds any `Send + Sync + 'static` type. A `#[system]` is shipped
  only in a boundary crate or a lint-file carrier (decision 5.18), and none of them may
  name this crate, a backend crate. Where that system and trait are declared, and which
  world runs the system once a frame, are `frame-design`'s; any other route takes a lock
  that section 3 would have to permit. A `GraphNode`'s `run` has `&self` and would need a
  lock to reach `&mut RenderGraph`, which section 3 does not permit on the per-frame path;
  a scope child holds a `ChildContext`, not a `JobContext`; and the main thread holds
  neither.
- **`graph-impl`: a frame of another graph.** `record` and `submit` refuse a frame another
  graph made (`InvalidData`, "Errors and drops"). Two `compile`s may precede a `record`,
  since a `CompiledFrame` borrows no graph, so the second may evict the first frame's plan:
  comparing a frame's plan with the cache's cannot tell an evicted plan from a foreign one.
  The graph tells them apart by a token of its own that every plan it compiles carries,
  such as an `Arc` compared with `Arc::ptr_eq`, and not by comparing cache pointers.

## A frame, by step

1. **Declare.** The caller builds a `FrameDecl`: imports, transients and passes, in order.
   Nothing reaches a device.
2. **Compile.** `RenderGraph::compile` hashes the declaration and returns a `CompiledFrame`:
   the declaration with the plan the cache held or, on a miss, the plan it derives, whose
   transients it makes.
3. **Record.** `RenderGraph::record`, called from a job, forks the frame's recording jobs and
   returns a `RecordedFrame` when all have finished.
4. **Latch.** The caller writes the buffers the frame reads last, such as the camera's,
   through the device, and flushes them. Nothing recorded depends on their bytes, and null
   judges a read at the submit, after the write. The host does not write a range a pending
   submit reads (rhi-api, "Buffers and host access"), so a latched buffer is one no frame
   still in flight reads: a buffer for each frame in flight, or a range of a ring.
5. **Submit.** `RenderGraph::submit` makes the frame's submits in order and returns the
   frame's point.

## Behaviour the signatures cannot express

### Declaring

- A `FrameDecl` is a value: it borrows no graph, its calls reach no device, check nothing and
  cannot fail. It owns a copy of every slice and label its calls are given and a box for
  each recorder. The contract bounds no allocation.
- Ids are numbers in the declaration that issued them: images from zero in declaration
  order, imports and transients in one sequence; buffers from zero; passes from zero,
  whatever their kind. An id carries no mark of its declaration. Used with another, it names
  whatever has its number there.
- `compile` reports every breach of a declaration as `ErrorKind::InvalidData`, naming the
  pass or the resource by its label. Labels need not be unique. The rules are these and no
  others:
  1. The view count is at least one.
  2. Every id a pass names is one the declaration issued.
  3. A pass names no image twice and no buffer twice, across all its lists.
  4. An import's `mips` and `layers` are at least one, and an attachment's `mip` and
     `layer` are inside the image's declared `mips` and `layers`.
  5. A transient is attached by its format: as `depth` where `Format::is_depth`, as colour
     otherwise.
  6. `sampled` names imports alone ("Not in this crate yet").
  7. A compute pass declares no buffer `BufferAccess::Index`.
  8. An import's `last.state` and its `handoff`'s state are not `Undefined`.
  9. An import's `last`, a buffer's `initial`, and an image's `initial` unless that is
     `Undefined`, are on a queue the graph records for: on a device whose present role
     has a queue of its own, `QueueRole::Present` is refused there. `handoff` may name any
     role.
- Everything else about a pass's shape is the device's to judge, by rhi-api's rules: a
  transient the device cannot make is returned by `compile` as the device returned it, and
  attachments of unequal extent, an import attached against its format, or more colour
  attachments than the device reports are refused at `submit` (`usage`).
- A declaration with no pass compiles. Its frame still makes one submit, which signals its
  point.

### Uses and order

- A use is all the graph knows of a pass:

  | Declared use | Writes | State the graph brings the resource to |
  |---|---|---|
  | `ColourUse` with `local_read: false` | yes | `ImageState::ColourAttachment` |
  | `ColourUse` with `local_read: true` | yes | `ImageState::LocalRead` |
  | `DepthUse` | yes | `ImageState::DepthAttachment` |
  | `sampled` | no | `ImageState::SampledRead` |
  | `BufferAccess::Read` | no | `BufferState::ShaderRead` |
  | `BufferAccess::Write` | yes | `BufferState::ShaderWrite` |
  | `BufferAccess::Index` | no | `BufferState::Index` |
  | `BufferAccess::Indirect` | no | `BufferState::Indirect` |
  | A copy pass's source | no | `CopySource` |
  | A copy pass's destination | yes | `CopyDestination` |

- The graph tracks one state for a whole image. A pass that attaches one mip of one layer
  brings every subresource to the attachment's state.
- A pass sees each resource as the passes declared before it left it. There are no versions
  to name and no ordering constraint to state: the edges the graph derives run from each
  pass to every later pass that uses a resource it writes.
- **Graph order is declaration order with the culled passes left out.** Every derived edge
  points forward in declaration order, so the stable topological order of the edges,
  declaration order breaking a tie, is declaration order itself, and the graph never
  moves a pass. `CompiledPlan::passes` is graph order.
- Every write is taken as partial. The graph does not reason that a clear or a
  `Load::DontCare` overwrites what an earlier pass wrote, in culling or anywhere else.

### Culling

- A pass is kept when it writes an import, or writes a transient that a kept pass declared
  after it uses. Every other pass is culled: one that writes nothing, and one whose every
  write is to a transient no kept pass later uses. A readback is a buffer import whose
  `last` is `HostRead`, so a pass whose only output is a readback is kept.
- A culled pass's recorder is never called and nothing is recorded for it. A transient no
  kept pass uses is never made, and `CompiledPlan::transient` returns `None` for it.
- Culling is part of the plan. A pass culled in one frame is culled in every frame of that
  declaration.

### Imports

- An import's `initial` is the caller's word for where the resource is before the frame's
  first submit. The graph does not check it, and on a device nothing does. Null refuses a
  first transition whose `from` is wrong (`state`).
- A frame leaves every import in its `last`, whether or not a kept pass used it: by a
  closing transition after its last use, or, where no kept pass uses it, by one transition
  from `initial` to `last`. An import no kept pass uses whose `initial` and `last` are equal
  as the layout resolves them has no command recorded for it, unless `handoff` asks for a
  release.
- `Undefined` as an image's `initial` discards its contents: the graph takes the image
  for the queue that uses it first, with no transfer, and `initial.queue` is ignored.
- `Undefined` as a buffer's `initial` says the state is not given, and `initial.queue` is
  then the queue the buffer was last used on, or any where it never was. The graph takes
  the buffer there first, which keeps its bytes (rhi-api, "States"), and moves it by a
  transfer like any other where the frame first uses it on another queue. So a buffer may
  be imported from `Undefined` in every frame, under one declaration from the first frame
  on, and its bytes are the frame's to read. Where the word is wrong the bytes are
  discarded, and null refuses the next read (`undefined-read`).
- Where a resource lives between frames is the caller's to say, through the queues of
  `initial` and `last`. A buffer that a compute pass alone uses, declared on the graphics
  role at both ends, is transferred out and back in every frame on a device whose compute
  role has a queue of its own; declared on the compute role, it is not.
- `handoff` is for a next user that is not this graph. Where `last` to `handoff` is a
  transfer between two queues, as the layout resolves them, of an `Exclusive` image, the
  graph records its release half on `last`'s queue, after every use of the image in the
  frame and whether or not a kept pass used it, and the next user records the acquire
  with equal values, over every mip and layer the import declared, in a submit that waits
  for the frame's point. Otherwise the graph records nothing for it and the next user
  records the whole change. The present thread's read of a finished target is the case
  (rhi-api, "The present thread"; decision 5.6): `last: ColourAttachment` on `Graphics`,
  `handoff: SampledRead` on `Present`.
- `ready` is for a use the graph did not make: a point every submit of the frame that
  names the import waits for, not the first alone, as each submit that uses an import
  another queue wrote or changed waits for it (below). A wait for a point already reached
  costs no GPU wait. The caller gives the point of the last submit outside this
  graph that used the import, where that submit may still be pending and the import is
  declared from `Undefined` or is `Concurrent`: rhi-api's rule that a change of state or
  owner waits for another queue's pending use (`no-qfot`) is met by this wait, and so is
  its rule that a use under another queue's pending write or change waits for it
  ("What `graph-impl` proves on null"). An `Exclusive` import needs a point as well where
  the caller brought it onto its queue, by a transfer or a take from `Undefined` of its
  own, after another queue's write or change of it: the point of that queue's submit, or
  of a later submit to that queue. That write or change binds every submit that uses the
  import while it is pending, and the caller's wait orders the caller's submit alone. An
  `Exclusive` import already on its queue, which no pending submit of another queue wrote
  or changed, needs none, and a point given where none is needed costs a wait that is
  already met. A target taken back from the present thread is imported from `Undefined`
  with `ready` the point the present thread's read signals (rhi-api, "The present
  thread"). The point is one a submit already made signals, or one the host signals: a
  submit that waits for a point only a later submit to its own queue signals never
  completes, and no backend detects it (rhi-api, "Queues by role"). A frame of the client
  gives no point the host owes: architecture section 5.3 gates no submit on a host signal,
  and on Apple a submit whose host wait is unmet for about five seconds loses the device.
  A test gives one, briefly, to hold a frame pending.
- The graph orders its own frames, with no word from the caller, who cannot give one: the
  graph chooses each pass's queue. A submit that uses an import which another queue's
  submit, of this frame or an earlier frame of this graph, wrote or changed the state or
  owner of waits for a point that submit, or a later submit to its queue, signals; so does
  a submit that changes the state or owner of an import which another queue's submit of an
  earlier frame used. A submit uses what rhi-api counts ("States"): a transition's
  resource, an attachment, a `sampled` import (the image of a declared slot), a declared
  buffer, a copy's side, an index or an indirect buffer. Each submit that uses such an
  import waits, not the first of its queue alone. For an `Exclusive` import the first such
  use on a queue is ordered by a transfer's wait or, for an image imported from
  `Undefined`, which moves to another queue with no transfer, by these waits before that
  transition. For a `Concurrent` one these waits are the only order. Null refuses a graph
  that leaves out the wait before a use, by rhi-api's `no-qfot`, and
  `use_in_a_later_submit_than_the_waiting_change_is_refused` with the cases beside it
  holds that ("What `graph-impl` proves on null"). The graph may make a frame's submit
  wait for an earlier frame's submits to another queue where it need not; which such
  waits it makes is not promised.
- One handle is imported once in a declaration. The graph cannot see a second import of it.
- An import is never an image made with a memoryless request: the graph stores every
  import's attachments ("Transitions and store actions"), and storing such an image is
  refused on every backend (rhi-api, `memoryless-store`).

### Transients

- A transient is an image the plan makes, on the miss that compiles the plan, under the
  label it was declared with, `Exclusive`, with the usage its kept uses need: `COLOUR` for
  a colour use, `DEPTH` for a depth use, `INPUT` for a colour use with `local_read`,
  `COPY_SOURCE` and `COPY_DESTINATION` for a copy pass's, and `TRANSIENT` with a memoryless
  request. It belongs to its plan. Every frame recorded from the plan uses the same image.
- Only render and copy passes use a transient, and both are the graphics role's ("Queues"),
  so a transient never leaves the graphics queue.
- A transient lives from its first to its last kept use in graph order. Its contents never
  survive a frame: every transient begins each frame `Undefined`, whatever its memory, by a
  transition from `Undefined` before its first use.
- Its memory is the plan's choice, reported by `CompiledPlan::transient`:
  - `Memoryless` where it has one mip and one layer, its only kept use is as an attachment
    of one pass, the device reports the request honoured for that attachment
    (`memoryless_depth` for a depth use; `memoryless_colour` for a colour use with
    `local_read`), and the pass has no colour use with `local_read` unless the device
    reports `memoryless_survives_local_read`. Such an attachment is never loaded with
    `Load::Keep`: a memoryless candidate declared so is given other memory, and null then
    refuses the read (`undefined-read`).
  - `Placed` otherwise, where `Device::image_placement` accepts its descriptor: at a byte
    range of a memory block of the plan's.
  - `Own` where `image_placement` returns `Unsupported`.
- **Aliasing.** Two placed transients share bytes only where the last kept use of one comes
  before the first kept use of the other in graph order. The plan places them in the order
  of their first kept use, declaration order breaking a tie, each at the lowest offset of
  the lowest-numbered block where its bytes, at its alignment, overlap those of no
  transient whose lifetime overlaps its own, and makes a block where none has room. How
  large it makes a block is its choice: at milestone 2 a plan has one block, sized to the
  high-water mark of its placements, and a device refuses a block past its largest
  allocation (`ResourceExhausted`), which `compile` returns. So a transient takes the
  bytes of one that has ended wherever those are the lowest that fit, and in a frame whose
  kept passes use two placed transients and no more, of equal format, extent, mips, layers
  and usage, whose lifetimes do not overlap, the two have one placement. Two whose
  lifetimes meet in one pass, the last use of one and the first of the other, overlap.
- Before the first use, in each frame, of a placed transient whose range another's
  overlaps, the graph records `Encoder::alias` for it. That covers the first frame, where
  rhi-api accepts the call though nothing came before, and every later frame, where the
  transient that last used the bytes is one of the frame before.
- On native Metal the one block is suspected to cost order: D158 fences placed images by
  block, so every encoder that uses a placed transient would wait for every earlier one
  that did, whether or not their bytes overlap. `rhi-metal` measures it; a block for each
  group of transients that share bytes is the remedy where it is so.
- **The graph does not judge a read before the first write.** A declaration whose pass
  keeps, copies from or locally reads a transient no earlier pass wrote compiles and
  records. Null refuses its submit (`undefined-read`); a device reads undefined bytes.
- An evicted plan's transients and blocks are destroyed once no `CompiledFrame` or
  `RecordedFrame` holds the plan and the last frame submitted from it has completed. The
  graph learns that by reading a timeline's value inside `compile`, `record` or `submit`,
  never by waiting, so a destroy is not refused (`pending`).

### Transitions and store actions

- Before each kept pass, in the pass's own job and encoder and inside its span, the graph
  records one transition for every resource the pass uses that is not already in the state
  and on the queue the use needs, from where the frame so far left it.
- Where the resource is already there and the state is one that writes (`ColourAttachment`,
  `LocalRead`, `DepthAttachment`, `CopyDestination`, `ShaderWrite`), and an earlier pass of
  the frame used it in that state or the use is the import's first in the frame and its
  `initial` is that state, the graph records the same-state transition rhi-api requires
  between the two ("States"). The second case is the frame's edge: null keeps a write
  across submits, and without it an attachment kept with `Load::Keep` every frame, or a
  buffer a compute pass writes every frame, is written after the last frame's write with
  nothing between them.
- A change between two queues of an `Exclusive` resource is recorded twice with equal
  values: the release on an encoder of the queue that used it last, after that use, in a
  submit made before the acquiring one; the acquire before the pass. The acquiring submit
  waits for a point the releasing one signals. A `Concurrent` resource's change is recorded
  once, and its submit waits for a point of every submit of another queue that uses the
  resource before it.
- After its last kept use, an import that is not in its `last` is brought there by a closing
  transition, then released toward `handoff` where that is asked.
- The graph derives each attachment's store action. An import's is `Store`. A transient's
  is `Store` where a kept pass declared later uses it, and `DontCare` otherwise; a
  memoryless one's is always `DontCare`. Load actions are the declaration's.
- The graph records no transition the declaration does not imply and omits none it does.
  On null a missing or a wrong one is refused (`state`), which is how `graph-impl` shows a
  build that drops one.
- **The graph never names a resource twice in one transition call.** The elements of one
  `transition_images` or `transition_buffers` call are not ordered with one another on a
  device (one `vkCmdPipelineBarrier2` on Vulkan), so where a resource has two transitions
  in a row, such as a closing transition and then the release toward `handoff`, or a take
  from `Undefined` and then a release, each is in a call of its own, in that order. The
  tests' logging device refuses a submit whose encoder had a call that named one twice.

### What a pass reaches

- A render or compute pass is begun with a `PassReach` whose `slots` are the declaration's
  `slots`, as given, and whose `buffers` hold one `BufferReach` for each buffer declared
  `Read` (`ShaderRead`) or `Write` (`ShaderWrite`). A buffer declared `Index` or `Indirect`
  is brought to its state and is not in the reach: the call that names it is its
  declaration (rhi-api, "What a pass declares").
- A buffer a shader reaches through an address that no `push_address` call wrote, one read
  out of another buffer or pushed as bytes, is declared `Read` or `Write` all the same.
- The heap is the caller's. The graph writes no slot and reads none. For each image in
  `sampled` the caller keeps a slot holding it, written outside the graph under rhi-api's
  rules (`uuwp-used`), and names the slot in `slots`. A slot that holds an image the frame
  does not declare is the caller's to keep in `SampledRead` for the pass, with no pending
  submit of another queue that wrote it or changed its state or owner: the graph cannot
  see what a slot holds, so it cannot add that wait. An image that may have one is
  declared, and named in the pass's `sampled`.
- A copy pass begins no pass on the encoder and has no reach.

### Queues

- Render and copy passes are the graphics role's. So is every compute pass on a device
  whose compute role shares the graphics queue (`QueueLayout::compute` is `Shared`): the
  plan then names `Graphics` for every pass and the frame's encoders are all that role's.
- On a device whose compute role has a queue of its own, a compute pass is the compute
  role's when it has no graphics-queue dependency, and the graphics role's otherwise. A
  compute pass depends on a graphics-role pass of the frame when both use one resource,
  unless the resource is an import made `Concurrent`, neither pass writes it, both need it
  in one state, and its `initial` is that state. No transition of it then falls inside the
  frame before both uses, and a closing transition after both waits for the other queue's
  use ("Transitions and store actions"). The rule is applied until nothing changes: a
  compute pass made graphics-role by it is a graphics-role pass for the others.
- The window of architecture section 12.3 is the frame. A dependency that crosses frames is
  an import's `initial` or `last`: a transfer at the frame's edge, which the graph records
  and which does not keep a pass off the compute queue. So what a compute-role pass writes
  is read on the graphics queue in a later frame, never in its own.
- The graph records for the graphics and compute roles alone and never submits to the
  present role (`present-queue`).

### Compile and its cache

- The key is a hash of, in this order: the view count; the device's `QueueLayout`; each
  image in declaration order (import or transient; its label; an import's `mips`, `layers`,
  `sharing`, `initial`, `last` and `handoff`; a transient's format, extent, mips and
  layers); each buffer in declaration order (its label, `sharing`, `initial`, `last`); each
  pass in declaration order (its kind, its label, and each use: the list it is declared in,
  which is `colour`, `depth`, `sampled`, `buffers` or one of a copy pass's four, the id, an
  attachment's mip, layer, `local_read` and the variant of its load, a buffer's access).
- Against architecture section 12.3's list: the pass list is the passes' kinds and labels;
  the edges are a function of the uses, which are hashed; the resource descriptors are the
  transients' and what an import declares; the imported initial states are `initial`, and
  `last` and `handoff` are hashed beside them, because the closing transitions are part of
  a plan; the view count is hashed by itself; the queue configuration is the device's
  layout. A graph has one device, so the layout never varies within one cache. It is in the
  key so that one declaration has different keys on a one-queue and a several-queue device.
- Not in the key, and free to change every frame under one plan: every handle, every
  `ready` point, every clear value, every `slots` list and every recorder.
- A hit is an equal declaration, not an equal hash: the cache compares what it hashed, the
  list of each use included, so a collision compiles again and two declarations that differ
  only in which list a use is in never share a plan. `CompiledFrame::fresh` is `true`
  exactly when this call compiled the plan.
- On a hit `compile` makes nothing. On a miss it calls `image_placement`,
  `create_memory_block` and `create_image` for the plan's transients, and after an error
  destroys what it made and caches nothing. Either way it may read a timeline's value and
  destroy what an evicted plan left ("Transients").
- The cache holds at most `GraphDesc::plans` plans. A miss into a full cache evicts the plan
  least recently returned by `compile`. An evicted plan lives while a frame holds it.
- A plan is a function of the declaration, the device's layout, its capabilities and what
  `image_placement` returns. The same declaration on the same device compiles to the same
  plan in every run.

### Recording

- `record` opens one scope on `ctx` and forks one child for each recording job, then returns
  when all have finished (`crates/task/README.md`, "Scopes"). The jobs run on the
  frame-critical workers, the calling worker among them, so `record` is called from a job.
  While another job's `JobContext::block_in_place` holds the executor's gate, the
  recording jobs run on the workers its reservation leaves (`crates/task/README.md`,
  "Workers"): the physics phase slows recording, by as much as verification's
  `record-stall` row measures, and stops it for the step where the reservation is zero, as
  it is where there is one frame-critical worker.
- The passes of one submit are split into the smaller of `GraphDesc::jobs` and their number
  of jobs, each a run of consecutive passes in graph order. Where the plan cuts the runs is
  its choice, part of the plan and the same in every run. A submit that carries transitions
  alone is one job.
- Each job has one encoder, of its submit's role, for the frame: one encoder, and so one
  command pool, for each recording job and frame in flight. An encoder is reused only after
  the point of the frame it was submitted in is seen reached, and is reset then
  (`pending`). The graph learns it by reading the timeline's value inside `record`, never
  by waiting; where no encoder is free it makes one. The graph does not bound the frames in
  flight: the caller's pacing does, and `GraphStats::encoders` shows the count.
- Each kept pass is recorded inside one span: target `gouda`, name `pass`, trace level,
  field `label`. Inside it, in order: the alias calls and transitions the plan derived, the
  pass's beginning with its attachments and reach, its recorder, its end. A pass's load and
  store actions happen whether or not its recorder draws. The graph opens no other span of
  that name: closing transitions and releases are recorded outside every pass's span, on
  an encoder of the queue the resource was last used on, after that use.
- A recorder is called once for a kept pass, on a frame-critical worker, by the one job
  that owns it, which is why it is `FnOnce + Send` and need be neither `Fn` nor `Sync`: it
  may consume what it captured and may hold a `&mut` for the frame. Recorders of one job
  are called in graph order; no order is promised between two recorders of different
  jobs, which may run at once. A recorder does not block or wait (architecture section
  5.1): not for another recorder, which on one worker runs after it, not for a lock, and
  not for a timeline point. Called or not, every recorder is dropped before `record`
  returns, on a worker that is not promised. A recorder reaches the encoder through its
  recorder type alone: it cannot transition, alias, begin or end a pass, or reset.
- A recorder call forwards to the `Encoder` call of its name with the handle imported under
  the id. An id resolves only as its pass declared it: a buffer for `push_address` was
  declared `Read` or `Write`, for `bind_index_buffer` `Index`, for an indirect argument or
  count `Indirect`; a copy's sides were declared as the source and destination they are
  used as. Any other id is a breach: `record` returns `InvalidData` naming the pass and the
  call, the first such pass in graph order where there are several, and no frame is
  recorded. Heap slots are passed on unchecked, and the device refuses one the pass did not
  declare (`undeclared`, every backend).
- A copy pass's recorder writes each destination in one call, which takes as many regions
  as it needs, every mip and layer of an image among them, no two of which write one byte
  of a buffer or one texel of an image. Vulkan forbids it
  (`VUID-vkCmdCopyBuffer-pRegions-12482`, `VUID-vkCmdCopyBufferToImage-pRegions-12483`,
  `VUID-vkCmdCopyImageToBuffer-pRegions-12485`), which region lands last is the driver's
  choice, and `bounds` refuses it on every backend (D160, and D158's addendum at
  `rhi-null`), as `copy_regions_overlapping_in_the_destination_are_refused` holds. A
  second call that writes a destination the pass already wrote is a breach like the
  others, reported as above: rhi-api requires a
  transition between two copies into one resource ("States"), and a recorder cannot record
  one. Copies into one destination from two sources are two copy passes, and the graph
  records the transition between them.
- A panic in a recorder aborts the process, as any scope child's does.
- After an error from `record` nothing of the frame is submitted, and its encoders are
  reset and free.

### Submission

- A frame's submits are the runs of graph order by queue: a new submit begins where the
  queue that serves a pass differs from the one before it. Before or after them come
  submits that carry transitions alone, where a release has no earlier run of its queue to
  go in or a closing transition has no later one. `CompiledPlan::submits` counts them all
  and `PlannedPass::submit` numbers them.
- **Submission order is graph order.** `submit` calls `Device::submit` once for each, in
  that order, each with its jobs' encoders in graph order. Passes of one queue therefore
  run in graph order. Under "Queues" no pass uses what a pass of the same frame on another
  queue uses in another state or writes, so the frame's queues are ordered against each
  other by three waits alone: a transfer's, where an `Exclusive` import crosses at the
  frame's edge; that of a submit that uses an import under another queue's write or
  change, or changes the state or owner of one under another queue's use, an image
  imported from `Undefined` that moves to another queue among them ("Imports",
  "Transitions and store actions"); and the frame's last submit's, for the frame's point.
- A frame all of whose commands are for one queue is one `Device::submit`. That is every
  frame on a device with one queue.
- The frame's point is a value of one timeline of the graph's. It is signalled when every
  submit of the frame has completed: the frame's last submit signals it, and waits for a
  point of each submit the graph made to another queue after the last submit it made that
  signals a frame point, which after a whole frame are the frame's own, and for that frame
  point where it is signalled from another queue (rhi-api, "Timelines"). Values grow with
  each frame submitted, so a point reached means that frame's submits, and every earlier
  frame's, have completed: those on the last submit's queue by submission order, and those
  on another queue by these waits.
- Across frames, under several queues, each submit of a frame waits for the last submit
  the graph made to the other queue before the frame, where it has made one: that covers
  every earlier frame's use, write and change there, and is how the graph makes the
  cross-frame waits of "Imports" (null credits a submit with its own waits alone). The
  frame's last submit also waits for the frame point before where that was signalled from
  the other queue. The wait for the other queue's point does not imply it, though one
  submit signalled both: the signals of one submit are unordered with one another on
  Vulkan, so only a wait for the frame point itself keeps the frame timeline's values
  growing (`VUID-VkSubmitInfo2-semaphore-03882`).
- The two cost the frame's ends. The last submit waits for the frame's work on the other
  queue before any of its own passes run, so the other queue's work overlaps only the
  submits made before the last one; and where a graph has used both queues, a frame's
  first submit on a queue starts after the other queue's last submit of the frame before,
  so no work of one queue overlaps the other's across the frame's end. A frame all of
  whose work is on one queue, on a graph that has used no other, makes neither wait, which
  is every frame of milestone 2: its work overlaps the frame before's on that queue as the
  device allows. A trailing submit with no encoder that signals the point is for a later
  milestone ("Not in this crate yet").
- `submit` waits for nothing and returns an error from `Device::submit` unchanged. A
  refusal of the frame's first submit changes nothing (rhi-api, "Timelines"): every import
  is as the declaration found it, and the frame's encoders are reset and free. An error
  from a later submit leaves the frame torn. The submits before it were made and stay
  made, the frame's point is not signalled until the graph's next `submit` (below), and
  its imports are in no state the graph vouches for. The error does not say which submit
  failed, so the caller treats every error from `submit` as a torn frame: it imports the
  frame's imports next from `Undefined`, relies on no buffer's bytes, and calls
  `History::reset` on every history whose halves the frame imported.
- A torn frame's planned last submit is never made: it is the frame's last, so the
  failed submit is it or comes before it. The graph's next `submit` therefore first makes
  that submit in the torn frame's place, with no encoders, to the queue the torn frame's
  last submit was planned for (rhi-api, "Recording and submitting": a submit with no
  encoders waits and signals alone). It waits and signals as that last submit would have,
  by the rule above, except that it waits for the frame point before wherever that was
  signalled: for the last submit made to the other queue, where there is one, which covers
  each submit the torn frame made there, and for the frame point before, and it signals
  the torn frame's point. Then the new frame is submitted. The torn frame's point is
  reached like any other frame's and the next frame's point is above it: the next frame's
  last submit waits for it where that submit goes to the other queue, and submission order
  covers the rest. The torn frame's submitted encoders and its plan are held, as a pending
  frame's are, until its point is seen reached, so nothing is reset or destroyed under a
  submit it made; the encoders of the submits it never made are reset and free at once.
- If that submit is refused, the new frame is not submitted: its encoders are reset and
  free, the graph is as it was before the call, still owing that submit, and `submit`
  returns the error unchanged, which the caller takes as a torn frame like any other. The
  call still waits for nothing: the submit in the torn frame's place is a wait on the
  device's queue, not on the host.
- A device with one queue never tears a frame: its frame is one `Device::submit`, which a
  refusal leaves unmade.
- A `RecordedFrame` dropped unsubmitted drops its encoders, none of which is pending.

### History

- `History` is the ping-pong primitive of section 12.3 and is plain data. The two resources
  are the caller's, made and destroyed outside the graph, and each frame imports them:
  `previous()` in the state the last frame left it, `current()` from `Undefined`.
- A handle is not in the key, so the swap changes no plan. A frame with no previous half
  declares one import fewer and has a key of its own: a history costs one plan for its
  first frame, or the first after `reset`, and one for every frame after.
- The owner calls `advance` after `submit` returned the frame's point. After a `submit`
  that failed it calls `reset` instead: the frame is taken as torn ("Submission"), so the
  half it read is imported next from `Undefined`, which discards it.

### Concurrency

- Every call that changes a `RenderGraph` takes `&mut self`. Compile, record and submit are
  one thread's at a time by the borrow, on any thread.
- **Shared across recording jobs, read only:** what the frame's declaration copied (its
  handles, clear values, slots and labels), the plan, and the device's `Arc`. **Owned by
  exactly one job:** its encoder, the recorders of its passes and the place its first
  breach is written, each lent to it as `&mut` from a slice split by job: a job's passes
  are consecutive in the declaration, so the split is by index. Nothing else exists while
  the jobs run, and nothing a job writes is reachable from another.
- **This crate takes no lock and has no `unsafe`, so section 3 needs no amendment for
  it.** The locks a frame takes are a device backend's (D158), under the device and
  encoder calls, which section 3 permits; the scope `record` opens takes none.
- `sync_view::SyncView` holds the graph's and a recorded frame's encoders and a
  declaration's recorders. It wraps a `std::sync::Mutex` for its `Sync` impl alone, which
  holds where the value is `Send`, as every value it holds is, and calls only `get_mut` and
  `into_inner`, which take `&mut` or the value, block nothing and take no lock.
  `std::sync::SyncView` (unstable `exclusive_wrapper`) does this job and is `Sync` for any
  value; it is not stable on the pinned toolchain.
- `RenderGraph`, `RecordedFrame` and `CompiledPlan` are `Send + Sync + 'static`.
  `FrameDecl<'f>` and `CompiledFrame<'f>` are `Send + Sync` whatever their recorders are,
  and `'static` where the recorders are. A recorder type is lent to one call on one worker.

### Determinism

- The crate is outside the determinism boundary and in `[server_denylist]`: no world holds
  it in hashed state, no system inside the boundary names it, and a server links none of
  it. Nothing it reports is fed back into simulation state: not a key, a count, a point or
  a timing.
- It has no `StableKey` and no entity. Its one ordering rule is declaration order, which is
  the caller's: a renderer that declares from a map declares in key order.
- A plan depends on no thread, address or hash-map order ("Compile and its cache"), and the
  commands a frame records depend on the plan, the declaration and the recorders alone. So
  two runs of one declaration submit the same commands in the same order, whichever worker
  recorded which job. That is what makes a device test's readback comparable, and gives
  null the same commands to judge in every run.

### Errors and drops

- `ErrorKind::InvalidData` is the graph's own: a zero in `GraphDesc`, a declaration's
  breach, a recorder's breach, and a `CompiledFrame` or `RecordedFrame` given to a graph
  that did not make it. Every other error is the device's, returned as the device gave it:
  `Unsupported`, `ResourceExhausted`, `DeviceLost`, and a refusal that names its rule.
- `RenderGraph`'s drop waits, with no bound, until every submit it made has completed,
  a torn frame's among them, then destroys its transients, blocks, timelines and encoders.
  After a torn frame with no `submit` since, it first makes the submit in the torn frame's
  place ("Submission"), then waits as above. An error from that submit is logged under
  `gouda` and dropped; the drop then waits for every point it can see signalled, drops its
  encoders, which waits for their submits unless the device is lost (rhi-api, "Handles and
  lifetimes"), and destroys what no pending submit uses. On null, which counts a submit
  pending until its point is seen, the torn frame's submits that signal no point stay
  pending, so what they used is leaked. This is the one drop outside an unwind that leaks.
  It is dropped off the frame-critical workers (rhi-api, "Handles and lifetimes"), and
  after the host has signalled every `ready` point a pending frame waits for that the host
  owes: the drop cannot release such a frame. On a lost device the wait ends at once.
- A drop made while the thread unwinds a panic does not wait. It reads the timelines,
  destroys what no pending submit uses, makes no destroy a pending submit would refuse,
  and drops its encoders, which leak what is pending, as rhi-api's drops do ("Handles and
  lifetimes"). It makes no submit, the one in a torn frame's place among them. So a test
  that fails holding a pending frame fails instead of hanging.
- A `FrameDecl` and a `CompiledFrame` drop as plain values and leave the graph as it was.

## What `graph-impl` proves on null, and the rule each proof rests on

`graph-impl` runs every row under both capability sets, `QueueLayout::ONE` and
`QueueLayout::SEVERAL`, where the row has a queue in it.

| What the graph must do | Null's rule that refuses a graph that does not |
|---|---|
| Record every derived transition, with the `from` the frame left | `state` |
| Record the same-state transition between two writers | `state` |
| Record the same-state transition before an import's first use in the frame where its `initial` is that writing state: one declaration, an import written in that state every frame, submitted for two frames | `state` |
| Record both halves of a transfer, the release submitted first, and wait for it | `no-qfot` |
| Wait before a change of state while another queue's pending submit uses the resource | `no-qfot` |
| Wait, in every submit that uses an import, a `sampled` one included, for each pending submit of another queue that wrote it or changed its state or owner, and for its `ready` point | `no-qfot`, where the submit that signals `ready` wrote the import or changed its state or owner: `use_under_another_queues_pending_write_is_refused`, `use_under_another_queues_pending_transition_is_refused`, `use_in_a_later_submit_than_the_waiting_change_is_refused`, `acquire_in_a_later_submit_than_the_wait_for_its_release_is_refused`, `take_of_a_mip_after_an_acquire_of_another_is_refused` and, for a `sampled` read, `sampled_read_under_another_queues_pending_write_is_refused` |
| Never record two regions of one copy call that write one byte or texel | `bounds`: `copy_regions_overlapping_in_the_destination_are_refused` |
| Record the alias call before the first use of a transient whose bytes another used | `no-fence` |
| Keep aliasing transients on one queue | `no-qfot` |
| Begin every transient undefined; store an attachment a later pass uses | `undefined-read` |
| Never store or keep a memoryless attachment | `memoryless-store` |
| Never ask for memoryless in a pass with a local read where the device loses it there | `split` |
| Declare in a pass's reach exactly the buffers its uses name, in the state each names | `undeclared`, `state` |
| Derive a transient's usage from its uses | `usage` |
| Place transients inside their blocks at aligned offsets | `bounds` |
| Reset an encoder only after its frame's point; destroy a transient only after its last frame's | `pending` |
| Signal growing values, and one timeline from two queues only across a wait | `timeline` |
| Give a submit only encoders of its role, each recorded once since its reset, with no pass left open | `encoding` |
| Never submit to the present role | `present-queue` |

The two rows above that waited on an amendment of rhi-api routed to the `rhi-null` lane
(D160) have their rules since that lane merged (D158, its addendum at `rhi-null`).
`no-qfot` refuses a use under another queue's pending write or change of state or
owner in every submit that makes one, not the first of its queue alone, an acquire
among them, and an acquire, a take from `Undefined` or a wait in an earlier submit
orders no later one; `bounds` refuses two regions of one copy call that overlap in the
destination. Null's conformance cases hold both in CI: the `Concurrent` path across
queues by `use_under_another_queues_pending_write_is_refused` and
`use_under_another_queues_pending_transition_is_refused`, the wait in a frame's later
submits by `use_in_a_later_submit_than_the_waiting_change_is_refused` and
`acquire_in_a_later_submit_than_the_wait_for_its_release_is_refused`, and a copy's
regions apart by `copy_regions_overlapping_in_the_destination_are_refused`. rhi-api's
text says that a re-take from `Undefined` ends a release no acquire met, which the
client's frame relies on ("What each consumer needs"), and
`acquire_of_a_release_a_retake_ended_is_refused` holds it.

Some proofs rest on this surface and not on a rule. An unreachable pass is culled and a
pass whose only output is a readback or an import is kept (`is_culled`, and a culled pass's
recorder never runs). Each input of the key, changed alone, forces a recompile (`fresh`,
`GraphKey`; the queue layout by one declaration on two devices; a use moved from one of a
pass's lists to another, such as a copy's image from its sources to its destinations).
A copy pass's second call into one destination fails `record`. Two transients alias
(`transient`), after which a read of the second before its first write is refused by
`undefined-read` above. Memoryless is asked for only where the device reports it honoured
(`transient` under a capability set with it and one without: a request the device does not
honour is not refused, it is given memory). No submit waits for a point that only a later
submit to its own queue signals: null does not refuse that, neither submit completes, so
the proof is every frame's point reached within a bounded wait under both capability sets.
The tests' logging device refuses a submit whose encoder had a transition call that named
one resource twice, whatever the device under it checks, over the five chains of two
transitions a frame can give one resource at its end. The waits for the frame point before
hold by the waits that device records, since null credits a wait for one signal of a
submit with all of them. Each queue rule's clause holds by the queue a pass
is given (`PlannedPass::queue`), and a wait inside the graph, such as a pending encoder's
drop, fails its test at a bound, with a message, rather than at nextest's kill.

## How each call is honoured

Every call reaches a device through rhi-api alone, so it is honoured on null and on native
Metal as the calls it makes are. The backend columns are those of rhi-api's own table ("How
each call is honoured"), whose evidence is cited.

| Call | What it calls | Null | Native Metal | Vulkan (MoltenVK on the M1) | Evidence |
|---|---|---|---|---|---|
| `RenderGraph::new` | `capabilities`, `create_timeline` | The set it was created with; a counter and a condition variable | A command queue for each role, no ownership; `MTLSharedEvent` | Four families on the M1, one queue on lavapipe; timeline semaphores | RHI items 1 and 8, points 2 and 11; Metal items 9 and 11, points 2 and 11 |
| `compile` on a miss: placed transients | `image_placement`, `create_memory_block`, `create_image` with `Placed` | Byte ranges for each block; which image last used each | `heapTextureSizeAndAlign`; an untracked placement heap | Bound memory at an offset, or `Unsupported` for an image the blocks' memory types do not take, which the plan gives memory of its own | Metal item 4, point 16 |
| `compile` on a miss: memoryless transients | `create_image` with `Memoryless`, only where `Capabilities` reports it | The request and whether its set honours it | `.memoryless` storage for colour and depth | Lazily allocated memory where offered | RHI item 7, point 4; Metal item 3, point 4 |
| `compile` on a hit | Nothing | | | | |
| `record`: its encoders | `create_encoder`, `timeline_value`, `Encoder::reset` | A command list and whether it is pending; refuses a reset while pending | A command buffer made on first use; `reset` drops it | A command pool and one command buffer; `vkResetCommandPool` | RHI item 6, point 8; Metal item 8, point 8 |
| `record`: around each pass | `alias`, `transition_images`, `transition_buffers` | The state and owner of each subresource, applied at the submit in order; refuses `state`, `no-qfot`, `no-fence` | Nothing, but for a placed image, whose every use the block's fence orders | Barriers of synchronisation 2, a same-state one included; a release and an acquire across families | RHI item 7, point 9; Metal item 4, points 9 and 16 |
| `record`: a pass | `begin_render_pass` or `begin_compute_pass` with the derived attachments and `PassReach`, `end_pass` | The attachments and what is declared | A render or compute command encoder; `useResource` for each declared slot's texture and each declared buffer, with its exact usage | `vkCmdBeginRendering`; the declaration unused | RHI item 7, points 5 and 10; Metal items 2 and 3, point 5 |
| `RenderRecorder` | `set_viewport`, `set_scissor`, `bind_graphics_pipeline`, `bind_index_buffer`, the pushes, `local_read`, the draws | The bound pipeline; typed pushes against the declaration; what was written since the last local read; checks states, declarations and capabilities and draws nothing | State on the encoder; nothing for the local read; `drawPrimitives`, `drawIndexedPrimitives`, one indirect record a call; no count form, which it does not report (`capability`) | Dynamic state; `vkCmdPushConstants`; the local-read barrier; the draw calls, multi-draw and indirect count where reported | RHI items 2 to 4, 7 and 9, points 10 and 12; Metal items 1 to 3 and 5, points 10 and 12 |
| `ComputeRecorder` | `bind_compute_pipeline`, the pushes, `dispatch`, `dispatch_indirect` | The bound pipeline; typed pushes against the declaration; checks, and runs nothing | `dispatchThreadgroups`, and its indirect form, on a serial compute encoder | `vkCmdDispatch`, `vkCmdDispatchIndirect`; a shader-write barrier between two dispatches of a pass that declares a `ShaderWrite` buffer | `dispatch`: RHI items 2 and 8, Metal items 2 and 9. `dispatch_indirect`: architecture 12.1 alone, as rhi-api's table says |
| `CopyRecorder` | `copy_buffer`, `copy_buffer_to_image`, `copy_image_to_buffer` | Buffer to buffer moves bytes; the image forms are checked | Blit encoder copies | The copy commands | RHI item 10, point 13; Metal item 12, point 13 |
| `submit` | `Device::submit` for each of the frame's submits, with waits and signals | Checks every command in order, applies the states, completes when its waits are reached; pending until the caller sees its point | Command buffers enqueued in slice order; `encodeWaitForEvent`, `encodeSignalEvent` | `vkQueueSubmit2` with timeline semaphores | RHI items 6 and 8; Metal items 8 and 9 |
| An evicted plan's end; `RenderGraph`'s drop | `timeline_value` or `wait_timeline`, then `destroy_image`, `destroy_memory_block`, `destroy_timeline`, and the encoders' drops | Refuses a destroy while pending | The objects released | The objects destroyed | rhi-api, "Handles and lifetimes" |

Native Metal has no ownership and no layouts (Metal point 9), so a transfer's two halves
and every transition but a placed image's are nothing there. The graph records them all the
same, because states are the caller's on every backend (D158), and what Metal reads from
this crate is the reach of each pass and the alias relation.

## Not in this crate yet

Each is absent because no lane of milestone 2 needs it. Adding one is an amendment of the
kind section 20 allows, a new field, variant or call, and changes what no call above means.

- **A transient sampled through the heap.** A slot holds an image by handle, a slot is not
  written while a pending pass declares it (`uuwp-used`), and an image is not destroyed
  while a slot holds it, so it needs a protocol between a plan's transients and the
  renderer's heap, which arrives with the bindless heap at milestone 3. Until then a pass
  reads a transient as an attachment it keeps, through the local read, or by a copy, and
  samples imports.
- **Transient buffers.** rhi-api places images in a memory block, not buffers.
- **A state for each subresource.** rhi-api puts a whole image in a slot, so no pass of
  milestone 2 samples one mip while it writes another.
- **Passes instanced by view.** The view count is hashed and reported. The caller declares
  each view's passes.
- **A point for each import.** The frame's one point waits for every queue, so a reader of
  the target waits for the frame's compute-queue work too. No pass of milestone 2 runs on
  the compute queue outside `graph-impl`'s tests.
- **A trailing submit for the frame's point.** A submit with no encoder, after the frame's
  last, that waits for the other queues' last submits and signals the point, so that the
  frame's last graphics passes need not wait for its compute work ("Submission").
  `CompiledPlan::submits` then counts one more for a frame with work on two queues, as it
  counts every submit; a later milestone that runs compute beside graphics adds it.
- **A reordering compile.** Graph order is declaration order. A compile that moves
  independent passes, to shorten a transient's life or to merge submits, changes
  `CompiledPlan::passes` and nothing a recorder sees.
- **A bound on frames in flight, and an encoder pool that shrinks.** The frame's pacing is
  `frame-design`'s.
- **A read-only depth use, storage images, a compute pass that writes an image.** rhi-api
  has none of them.
- **A hand-off of a buffer.** Every buffer's next user outside the graph is the host, which
  needs no transfer.

## Proof

There is nothing to run against a facade but `History`. `tests/consumers.rs` compiles what
each consumer writes: `graph-impl`'s proofs on null (aliasing, culling and null's refusal;
the key's inputs and the cap, and a use moved between lists; a write in one state across
the frame's edge; a copy's second call into one destination; both queue paths; encoders
under a lagging device), the minimal renderer's scatter and triangle passes through a
pass-provider trait and its readback, the client's frame through a `dyn` object with the
latch between record and submit and the target's hand-off to the present thread, and a
history's frame.
`tests/values.rs` runs the ping-pong and holds this file's surface table to the crate's
exports. The static assertions in `src/lib.rs` hold `RenderGraph`, `RecordedFrame` and
`CompiledPlan` to `Send + Sync + 'static`, `FrameDecl` and `CompiledFrame` to `Send + Sync`
whatever their recorders borrow, and every declaration and report type to plain data.

`graph-impl` owes the rows of "What `graph-impl` proves on null", each with the build that
fails it. On devices, by class of hazard (decision 5.17), it owes the dropped barrier on a
tracked attachment or buffer, which synchronisation validation reports; a host write under
a pending reader, which yields a wrong readback with the GPU made to lag; a host access
under a pending writer, refused on every backend (`pending`); and the encoder reset held
back with the GPU made to lag (`VUID-vkResetCommandPool-commandPool-00040`). Every device
run, the `gpu (lavapipe)` job's included, sets the Khronos layer's
`VK_KHRONOS_VALIDATION_SYNCVAL_LOAD_OP_AFTER_STORE_OP_VALIDATION=true` beside
`VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true`: without it the layer does not report a load
after a store between two render pass instances, which is the write across the frame's
edge of "Transitions and store actions" (verification, the row of D160 on the layer).

It owes three tests of recording itself, because none of the above fails a `graph-impl`
that records its jobs one at a time:
- **Recording runs in parallel.** On at least two frame-critical workers, a recorder of one
  job and a recorder of another each wait, with a bound, for the other, so the test fails
  rather than hangs where the jobs do not run at once, and each job's commands are still
  in graph order. The bounded wait is the test's own breach of "Recording"'s rule that a
  recorder does not wait, made to detect serial recording.
- **Chaos.** The same frame records the same commands under `ExecutorBuilder::chaos`
  (`ChaosConfig::shuffle_steals` and `inject_yields`, several seeds), at one and at four
  frame-critical workers.
- **Spans.** A test counts the spans of a frame against its kept passes, with its
  subscriber installed process-wide (`tracing::subscriber::set_global_default`, as
  `crates/task/tests/spans.rs` does): the jobs' spans open on other workers, so a
  subscriber set for the test thread alone sees the opener's only, and the count passes
  only where recording is serial.

The graph facade's concurrency review (round 1) built the first two over a model of
`record` on the shipped executor, with a switch that makes recording serial: under it every
check of order, breach and encoder reuse passed, and only the rendezvous failed, at its
bound. `reviewer-concurrency` reads the recording jobs' sharing against "Concurrency".
