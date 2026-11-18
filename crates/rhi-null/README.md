# lactose-rhi-null

The null backend of `lactose-rhi-api`: a device that draws nothing, keeps the state a caller
carries between calls, and refuses every breach of the contract by the rule it breaks
(`Rule::refuse`), under the `Capabilities` it is created with. Test support: the render graph,
the renderer and the present thread run their tests against it, under one queue and several,
coherent memory and not, and each optional capability on and off.

| | |
|---|---|
| Tier | backend, `[test_support]` and `[server_denylist]` in `crate-tiers.toml` |
| Determinism boundary | outside |
| Unsafe | none, `#![forbid(unsafe_code)]` |

**Status: implemented.** Every contract case of `lactose-rhi-api`'s conformance suite passes
here under five capability sets (`tests/conformance.rs`), and the output cases are skipped,
as null renders nothing. `crates/rhi-api/README.md` is the contract; null checks every rule
in its rules table, both columns, and this file states only what the contract leaves to null.

## The public items

| Item | What it is |
|---|---|
| `NullDevice::new(DeviceRequest, Capabilities)` | A device that reports `Capabilities` and honours exactly what they allow. The request's adapter is ignored; its surface, when there is one, lets the device make a swapchain; its cache starts empty, loaded or rejected as `open_cache` reads it; its sink is held and never called, since null has no validator and sends nothing of its own |
| `NullDevice::control()` | A `Control`, cloneable and holding the device weakly, that outlives it |
| `Control::lose()` | Marks the device lost: every later call returns `DeviceLost`, and every wait in progress returns. `false` once the device is dropped |
| `Control::stall(on)` | While on, presents do not complete: their submits run, and the driver's read of each image does not end, so an acquire finds no free image and a resize past a present is refused `pending`. `false` once the device is dropped. A device dropped while stalled is unstalled first |
| `NullSurface` | A surface with no window, for a device that makes a swapchain; its handles are unavailable |

The swapchain has three images of `Bgra8Unorm`. `acquire` and the present's wait for an
undecided point are bounded at one second.

## What null counts and runs

- A submit is pending until the caller has seen a point it, a later submit to its queue, or a
  submit that waits for such a point signals (`crates/rhi-api/README.md`, "Queues by role").
  A submit runs as soon as its waits are reached and the submit before it on its queue has
  run: its buffer-to-buffer copies move their bytes then, and its signals grow their
  timelines. Nothing else of a submit has an effect on bytes: an image holds none, and only
  whether its contents are defined is kept.
- A role is served by the queue `QueueLayout::serving` names, and records are kept by queue,
  not by role.
- A resource's record keeps three stamps for each queue: the last submit that names it, the
  last that writes it or changes its state or owner, and, of a buffer, the last that writes
  it. A host read and an invalidate wait for the second, a host write for the third. The
  `no-qfot` use rule compares the second with what the using submit's own waits order it
  after, through the submits they name and those submits' waits, and not through the
  earlier submits of its queue. An acquire marks the other queues' changes it ordered the
  resource after, and those bind no later use. The stamps are of the whole resource, not of
  each subresource: a `Concurrent` image changed at one mip on one queue holds back a use of
  another mip on another, and an acquire of one subresource discharges the rest.
- `read_buffer` reads `Readback` memory, of a buffer in a host-read state, whose written range
  has been invalidated where `coherent_host_memory` is `false` (`unflushed`).
- A SPIR-V module is checked by its header and its `OpCapability` and `OpExtension`
  instructions; nothing else of it is read. An `Msl` target is refused `Unsupported`.

## Locks

One `Mutex<Records>` holds every record: each resource's state by subresource and by queue,
its stamps, the heap's slots, the samplers, each queue's pending submits and the timelines'
values and marks. The other locks are taken around it, in one order: a queue's lock or the
heap's, never both; then the records'; then one entry's, a handle's in a table or a heap
slot's; then a buffer's bytes. Recording takes an entry's lock alone, never the records'. No
lock is held across a wait or the caller's closure: a host access lends its range under the
records' lock, runs the closure with none held, and clears the range in a second hold; a host
signal marks its timeline in one hold and folds its value in a second; a heap write holds the
heap's lock across both of its holds and the slot's write between them, so two writes of the
heap never interleave. A unit test's hook that runs a submit inside a heap write holds the
heap's lock and then a queue's, the one exception, and no cycle can form: a submit and a
present write no slot, so nothing takes the heap's lock under a queue's.

The drops that wait: an encoder's, for its pending submit; the swapchain's, for its
presents' submits; and the last of the device and every encoder and swapchain it made, for
every submit the device took. Each waits until the device is lost, if it is first, and none
waits while its thread unwinds a panic.

A present takes its queue's lock and the records', checks the submit, and walks the records:
each point its waits name is decided by the least pending promise at or above it, released
once that submit's own waits are and every earlier submit of its queue is. A walk that comes
back to a submit it is still deciding leaves the point undecided; a submit decided in one
branch is decided in the next. On an undecided point the present gives both locks up, waits
for the point's timeline to reach it, to be destroyed, or for the device to be lost, and walks
again; past the bound it is refused `pending`, and a timeline destroyed meanwhile has the next
walk refuse it `destroyed`, with nothing submitted.

## Tests

- `tests/conformance.rs`: every case of `CASES`, one test each, under the five sets
  `SETS` names: `LEAST`; several queues with every optional capability, memoryless kept
  across a local read and memory not coherent; compute on its own queue and present shared,
  with memoryless colour lost across a local read; compute shared and present on its own
  queue, with multi-draw indirect and 64-bit atomics but no count draw, and memory not
  coherent; and MoltenVK 1.4.2's shape on the M1, multi-draw indirect with no count draw and
  memoryless depth alone, lost across a local read.
- `tests/null.rs`: the device loss injected and observed at every call and wait; the
  invalidate a read needs where memory is not coherent; the cache's three starts; a handle
  whose index a later buffer took, which resolves nothing; four refusals null alone makes,
  each by its rule; and two
  races from several threads, submits on two queues racing a present, each round of which
  fails with a message past three refusals, and a destroy racing a present's wait.
- `tests/wakes.rs`: the wakes null alone makes, each bounded by how soon its waiter returns
  (10 s for a 60 s wait, 600 ms for a present or an acquire bounded at a second): a
  timeline's destroy under a wait; a loss, a destroy and a frame submit on its shared queue
  during a present's wait; a present waking a wait; an unstall waking an acquire.
- `src/tests.rs`: fixed interleavings, a call made in full between another call's two holds
  through a test-only hook (a host signal, a heap write, a host lend, a present's wait),
  among them a second heap write started inside a first, which the heap's lock holds back,
  and host signals landed inside each other's holds in both orders; the present's walk
  against queues held from running; and the drops that wait, of the swapchain with a present
  pending and of the device's last handle.
