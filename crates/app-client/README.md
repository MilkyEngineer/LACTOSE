# lactose-app-client

The client binary (architecture section 2): a local world shown through the render graph,
the renderer and the presentation schedule on a Vulkan device made from a window, presented
by the platform's present thread; and decision 5.1's fixed-frame mode, whose last frame is
read back, dumped and held to a CPU reference. A library holds the two objects
`lactose-app`'s shown frame is handed and the wiring, and `main` calls it, so that criterion
3(c)'s recorded test links what ships (`crates/app/README.md`, "What `lactose-app-client`
wires").

| | |
|---|---|
| Tier | leaf |
| Determinism boundary | outside; declares nothing a world registers, so it carries no lint file (decision 5.18, D152). A feature root and a harness package (`tools/xtask/ci-policy.toml`) |
| Unsafe | none, `#![forbid(unsafe_code)]` |

## What it holds

| Item | Contents |
|---|---|
| `ClientRender` | The `RenderSide`: the graph, the renderer and the presentation schedule over two offscreen targets of `TARGET_FORMAT`, each made at the frame's extent and replaced when it changes, the old image destroyed once its last draw and the present thread's last read of it are reached. `draw` uploads the remote batch, prepares, imports the target from `Undefined` behind `Draw::ready` with its hand-off to the present role, declares the renderer's passes, compiles and records; `submit` latches the camera from the `LatestSample` and submits. With a readback extent, a frame at that extent also copies its target and the renderer's instance records into buffers of its own, in a copy pass of its own declaration after the triangles and before the hand-off. `last_frame` reads the last submitted frame's pixels and records, its latched camera and fraction, and the placements extract handed the renderer for it (what the client's recording sink mirrors of the sink's contract, `crates/present-api/README.md`, "What the sink keeps"), where that frame was at the readback's extent; `None` where it was not, or no frame was submitted |
| `ClientPresent` | The `PresentSide`, the state of the present thread's lane: the swapchain, made beside the device; the present pass of `shaders/client/present.slang`, which samples the target through a heap slot (`HEAP`, slots 0 and 1) over the whole acquired image; a timeline of its own whose next value each present signals as its `read`. A resize behind the last read, a slot written behind it when the image is new, the acquire, the acquired image kept for the next call wherever a call fails after its acquire with the device not lost, at its encoder or at a present refused by any rule (a refusal as pending is that call's to retry, any other fails the frame), a resize that fails with a driver's error forgetting the acquired image, which the driver gave up (`lactose-rhi-api`, `Swapchain::resize`), `Stale` taken as a submit. Its count, `PresentReport::waiting`, the count the frame sheds by, is taken as a call begins: of the presents made before it, those whose read is reached and that no display-time report has named, neither they nor a later one; `None` where the swapchain has no source of display times. `close` waits, within a bound, for its last read, empties its slots, drops the swapchain and destroys its pipeline, sampler and timeline, logging each refusal at `warn` |
| `GAME`, `InMemory`, `Frames`, `REGION` | The game: `game-sim`'s registration with `game_sim::scene` as its sessions, so a join spawns the scene's seven orbiting rows. The cooked content of one region, an empty baseline and one asset, in memory; the input of each tick, the region and player 1's join in the first |
| `CookedShaders`, `PipelineCache` | The five stages read from the cooked shaders' directory (`<source>.<entry>.spirv`); the pipeline cache file, read into the device's request and written back, by a rename over the old file, once the renderer's pipelines are made |
| `wire`, `Wired`, `PresentBarrier` | Steps 4 to 6 of "What `lactose-app-client` wires" on any device and swapchain: what the binary and the null tests both run. `Wired::barrier`, which the shutdown does not need, holds the present lane to one use: `PresentBarrier::passed` queues a no-op there, which runs once every earlier call has been replied to and which the null tests await before they name a step |
| `run`, `Options`, `Mode`, `Outcome` | `main`'s run: the window and its pump on the main thread; on a driver thread of its own the executor, the pack set mounted (D136), the device and the swapchain in a main-lane call, `wire`, the frames, and the shutdown in the order of `crates/platform/README.md`, "Shutdown", on every path that made a device |
| `reference` | Decision 5.1's CPU reference (`MARGIN`, `reference`, `compare`, `Counts`, `surely_drawn`, `dump`) |

## Usage

```text
lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] [--seconds N]
lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] --fixed FRAMES DUMP
```

`DIR` for `--shaders` is the cooked shaders' directory, which `cargo xtask gpu` writes under
`target/gpu/shaders`. `--packs` names a directory of cooked packs, each mounted at start;
nothing in them is read at milestone 2, since the scene enters by `game-sim`'s session
applier and its mesh is the renderer's one triangle. No test or run here passes `--packs`:
the mount is `lactose-asset`'s `pack_set` and `Store::mount`, proven in that crate, and with
nothing read from a pack no frame here would show it. A flag given twice, or `--seconds`
with `--fixed`, is refused as malformed. The cache file is `pipeline-cache.bin`
in the working directory unless named. Paced, the run ends when the window is closed or
after `N` seconds. `--fixed` runs `FRAMES` steps at 512 by 384 and writes the dump into the
directory `DUMP`. The run prints its `Outcome` and exits 0; it exits 1 with the error, or
where a validator sent any message, and 2 on a malformed command line. The device is the
loader's first; the Vulkan loader's variables (`VK_DRIVER_FILES`, `VK_LAYER_PATH`,
`VK_INSTANCE_LAYERS`) choose the driver and the layers.

| Variable | Effect |
|---|---|
| `LACTOSE_CRASH_DIR` | The crash directory, as `lactose-app-headless`'s |

## The fixed-frame mode (decision 5.1)

A driver thread of the binary's own steps `Shown::step` with the frames' input, a fixed
extent (`FIXED_EXTENT`), a fixed fraction (`FIXED_FRACTION`) and a fixed sample
(`FIXED_SAMPLE`: no yaw, 0.5 radians down), then waits for the frame's point within ten
seconds, so that every step draws; a step that does not draw fails the run. After the last,
`Shown::stop`, the present thread's stop, and the render side taken back as its own type:
`last_frame` reads the target's pixels and the instance records the frame drew from, which
`reference::dump` writes as `last-frame.rgba` and `last-frame.txt` beside the fraction and
the latched camera. `reference::reference` rasterises the placements extract handed the
renderer with the camera on the CPU, as the vertex stage computes them, and
`reference::compare` holds every pixel to it: inside one triangle by `MARGIN` its slot's
colour, outside every triangle by `MARGIN` the clear colour, within `MARGIN` of an edge
either. `MARGIN` is a quarter of a pixel; its reasoning is the constant's documentation. The
scene is seven rows a seventh of a period apart on a circle of four metres, seen from 14
metres at 0.5 radians down; at any tick that is a multiple of 30 every triangle is at least
about 13 degrees from edge-on, and 600 is.

## Rules a caller should know

- Every frame and present call runs through `lactose-app`; nothing here reaches a world.
  The package declares no component, system, resource or applier; `game-sim` does.
- It is a feature root and a harness package: its boundary feature tree, with its
  dev-dependencies, must equal the other roots' (`cargo xtask features`).
- `wire` and the shutdown are called off the workers: the present thread's stop and the
  render side's drop block, each within a bound. `exit` is queued as the frames end, and
  before step 5 on every path that made a device, a start that failed before the frames
  among them, so a stall after it is under the pump's warning. The render side's drop runs
  on a thread of its own, made through `Executor::platform`, within 10 s (step 4 of
  `crates/platform/README.md`, "Shutdown"); past it the thread and what it holds are left,
  which keeps the window, so the pump never returns: in either mode the run logs the error
  and ends the process with exit status 1, its teardown not run. A fixed frame whose point
  is not reached within 10 s stops the world and fails the run the same way.
- The present side samples the targets through the heap's first two slots (`HEAP`). The
  heap's slots are the device's, shared by every user of it, and milestone 2 has no
  allocator for them: the renderer writes no slot, and a user beside the present side takes
  slots from the third on.
- The device is asked for the heap and sampler table the cooked shaders are compiled for
  (`HEAP_SLOTS_DEFAULT`, `SAMPLERS_DEFAULT`), since a pipeline's heap arrays must match them.
- A validator's message fails the run: `Outcome::messages` counts them, each logged at
  `warn` under `LOG_TARGET`.

## What proves it

| What | How |
|---|---|
| Criterion 3(c) | `tests/recorded.rs`, `a_shown_world_on_null_hashes_as_the_world_unshown`, listed in `[harness]` with its `::physics` stream: 48 steps of the scene on `lactose-rhi-null` with one queue, through drawn, hidden and held frames and the extent going E1, hidden, E2, E1, each step beside the same world built by `build_server` and stepped by `Server::step`; the hashes are equal at every tick, and the shown world's, with its physics snapshot's, are recorded under the runner's session |
| The run on null | `tests/null.rs`, on one queue and on several, through a device and a swapchain that note every refusal: the extent E1, hidden, E2 and E1 again, every frame drawn and queued; presents stalled over E1, E2 and E1 again, which holds frames on one queue and drops them with a present queue of its own, and sheds none, since null reports no display time; the stall let go and frames queued again; nothing refused but the presents and resizes null holds as pending past the stall, and no message. A step whose name is asserted waits until every present call so far has made its last swapchain call and then for a no-op queued on the present lane behind them, which runs once each has been replied to (`Run::settled`), so no name rests on a pace. With every second present call's acquire answered with no image, so that the call makes no submit and, on several queues, its target's release meets no acquire, the frames that draw that target again take it back from `Undefined` and null refuses nothing (verification row 230), each step settled so every frame but the hidden one is queued. Every target the render side made, at E1, E2 and E1 again, is destroyed (`retired-never-destroyed`); with the make of the new target at the change to E2 refused, that frame is torn, every later one drawn, and every target made destroyed, nothing refused (`r3-m3-slot-kept`, the retired image left in its slot). With every present's read kept pending until a wait sees it, on each layout, each frame that takes a target back waits for its last read, and null refuses nothing (`render-no-ready`). With the fixed-frame mode's readback at E1, the last frame's placements, those extract handed the renderer, hold the scene's seven instances (`scene-six-rows`, a join suppressed), and a last frame at E2 reads none (`readback-any-extent`); null runs no shader, so the device run holds the records to them. A clean shutdown logs no warning on any thread, its capture the process's subscriber (the renderer's drop's wait restored fails it, and a present side's `close` that destroys its timeline twice, which warns on the present thread). A lost device ends the next step with `DeviceLost`, and what the failure kept shuts down. A frame that waits for a region and one torn by a refused submit are not reached on null, whose control offers a loss and a stall of presents alone: `lactose-app`'s `tests/frame/states.rs` drives both over a scripted render side |
| The present side | `tests/present.rs`, `ClientPresent` alone on null with targets the test draws, at E1, E1, E2, E2, E1, E1 in turn: the swapchain resized at the first call and at each change of extent and at no other, and each image a slot held destroyed, which null refuses while a slot holds it, once its successor's read is reached. Through a device that notes every refusal: on several queues, each target released toward the present role by a frame nothing has observed, the present's submit waits for it (`present-no-wait`); with every present's read kept pending, targets 0, 1 and 0, each a new image at one extent, the third call's slot written behind the first read (`slot-no-wait`, `uuwp-used`); a present refused as pending made by the next call with the image it acquired (`pending-present-forgotten`), and one refused by another rule, or a present encoder that cannot be made, failing its call and made by the next call the same way (`refused-present-forgotten`, `encoder-fail`); targets 0 and 1 drawn again behind their reads, the second's present refused as pending, then a new image in slot 0, whose write waits for the last read, not the one before it (`slot-wait-stale`); a resize into a full image table failing with `ResourceExhausted`, after which the next call acquires anew (a side that kept the given-up image is refused, `Rule::Swapchain`); and `close` destroys the sampler it made, with no refusal |
| The count | `src/present.rs`'s unit tests, the count over scripted display-time reports and timeline values: no source; a present still behind the GPU; a report that covers every earlier present; the count rising one a frame; a report of an id never made here |
| The reference and the command line | `src/reference.rs`'s unit tests: the clear colour refused at a pixel inside one triangle near another's edge, and accepted near an edge inside none; the records held to the placements slot for slot and bit for bit, a moved placement, an offset's zero written negative, two slots swapped, a live slot unwritten, one live that extract left free and a record short each refused. `src/run.rs`'s: step 4's drop waited for within its bound and left past it (`drop-unbounded`). `src/main.rs`'s: a flag given twice, and both modes, refused |
| Both layouts under synchronisation validation | By hand, a probe build of a scratch copy (each present call sleeps 40 ms, every 13th makes no submit, every 97th draw is torn), paced for 20 s with the Khronos layer's synchronisation and thread-safety validation: under KosmicKrisp, whose window device serves the present role from the graphics queue, frames held, calls that made no submit and torn frames, no message; under MoltenVK, whose window device gives the present role a queue of its own, frames dropped, calls that made no submit and torn frames, no message (verification row 230) |
| Criterion 1 | The binary's fixed-frame mode, 600 frames, on the M1 under MoltenVK and KosmicKrisp with the Khronos layer, and under MoltenVK with Metal API and Metal shader validation, each validator's canary in its own process, with no line at `warn` or above: the records the device's scatter wrote equal the placements extract handed the renderer, the scene's seven instances are live and each is surely drawn, and every pixel is as the reference rasterised from those placements says. Decision 5.1's four mutations each fail it at its first wrong pixel; the scatter writing nothing, or slots 0 and 1 each written with the other's row, at slot 0's record; a join suppressed at the live count; and a scene of six rows at an instance surely drawn nowhere, under both drivers. Under both the last frame has 1,044 pixels surely drawn, 195,304 surely clear and 260 within the margin of an edge. Run by hand under the GPU lock (`docs/verification.md`); the Windows laptop's run is owed |
