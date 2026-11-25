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
| `ClientRender` | The `RenderSide`: the graph, the renderer and the presentation schedule over two offscreen targets of `TARGET_FORMAT`, each made at the frame's extent and replaced when it changes, the old image destroyed once its last draw and the present thread's last read of it are reached. `draw` uploads the remote batch, prepares, imports the target from `Undefined` behind `Draw::ready` with its hand-off to the present role, declares the renderer's passes, compiles and records; `submit` latches the camera from the `LatestSample` and submits. With a readback extent, a frame at that extent also copies its target and the renderer's instance records into buffers of its own, in a copy pass of its own declaration after the triangles and before the hand-off. `last_frame` reads the last submitted frame's |
| `ClientPresent` | The `PresentSide`, the state of the present thread's lane: the swapchain, made beside the device; the present pass of `shaders/client/present.slang`, which samples the target through a heap slot (`HEAP`, slots 0 and 1) over the whole acquired image; a timeline of its own whose next value each present signals as its `read`. A resize behind the last read, a slot written behind it when the image is new, the acquire, a present refused as pending kept for the next call, `Stale` taken as a submit. `close` waits, within a bound, for its last read, empties its slots and drops the swapchain |
| `Waiting` | `PresentReport::waiting`, the count the frame sheds by, taken as a call begins: of the presents made before it, those whose read is reached and that no display-time report has named, neither they nor a later one; `None` where the swapchain has no source of display times |
| `GAME`, `InMemory`, `Frames`, `REGION` | The game: `game-sim`'s registration with `game_sim::scene` as its sessions, so a join spawns the scene's seven orbiting rows. The cooked content of one region, an empty baseline and one asset, in memory; the input of each tick, the region and player 1's join in the first |
| `CookedShaders`, `PipelineCache` | The five stages read from the cooked shaders' directory (`<source>.<entry>.spirv`); the pipeline cache file, read into the device's request and written back, by a rename over the old file, once the renderer's pipelines are made |
| `wire`, `Wired` | Steps 4 to 6 of "What `lactose-app-client` wires" on any device and swapchain: what the binary and the null tests both run |
| `run`, `Options`, `Mode`, `Outcome` | `main`'s run: the window and its pump on the main thread; on a driver thread of its own the executor, the pack set mounted (D136), the device and the swapchain in a main-lane call, `wire`, the frames, and the shutdown in the order of `crates/platform/README.md`, "Shutdown", on every path |
| `reference` | Decision 5.1's CPU reference (`MARGIN`, `reference`, `compare`, `Counts`, `surely_drawn`, `dump`) |

## Usage

```text
lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] [--seconds N]
lactose-app-client --shaders DIR [--packs DIR] [--cache FILE] --fixed FRAMES DUMP
```

`DIR` for `--shaders` is the cooked shaders' directory, which `cargo xtask gpu` writes under
`target/gpu/shaders`. `--packs` names a directory of cooked packs, each mounted at start;
nothing in them is read at milestone 2, since the scene enters by `game-sim`'s session
applier and its mesh is the renderer's one triangle. The cache file is `pipeline-cache.bin`
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
the latched camera. `reference::reference` rasterises the records with the camera on the
CPU, as the vertex stage computes them, and `reference::compare` holds every pixel to it:
inside one triangle by `MARGIN` its slot's colour, outside every triangle by `MARGIN` the
clear colour, within `MARGIN` of an edge either. `MARGIN` is a quarter of a pixel; its
reasoning is the constant's documentation. The scene is seven rows a seventh of a period
apart on a circle of four metres, seen from 14 metres at 0.5 radians down; at any tick that
is a multiple of 30 every triangle is at least about 13 degrees from edge-on, and 600 is.

## Rules a caller should know

- Every frame and present call runs through `lactose-app`; nothing here reaches a world.
  The package declares no component, system, resource or applier; `game-sim` does.
- It is a feature root and a harness package: its boundary feature tree, with its
  dev-dependencies, must equal the other roots' (`cargo xtask features`).
- `wire` and the shutdown are called off the workers: the present thread's stop and the
  render side's drop block, each within a bound.
- The device is asked for the heap and sampler table the cooked shaders are compiled for
  (`HEAP_SLOTS_DEFAULT`, `SAMPLERS_DEFAULT`), since a pipeline's heap arrays must match them.
- A validator's message fails the run: `Outcome::messages` counts them, each logged at
  `warn` under `LOG_TARGET`.

## What proves it

| What | How |
|---|---|
| Criterion 3(c) | `tests/recorded.rs`, `a_shown_world_on_null_hashes_as_the_world_unshown`, listed in `[harness]` with its `::physics` stream: 48 steps of the scene on `lactose-rhi-null` with one queue, through drawn, hidden and held frames and the extent going E1, hidden, E2, E1, each step beside the same world built by `build_server` and stepped by `Server::step`; the hashes are equal at every tick, and the shown world's, with its physics snapshot's, are recorded under the runner's session |
| The run on null | `tests/null.rs`, on one queue and on several: the extent E1, hidden, E2 and E1 again, every frame drawn and queued; presents stalled, which holds frames on one queue and drops them with a present queue of its own, and sheds none, since null reports no display time; the stall let go and frames queued again; nothing refused and no message. With every second present call's acquire answered with no image, so that the call makes no submit and, on several queues, its target's release meets no acquire, the frames that draw that target again take it back from `Undefined` and null refuses nothing (verification row 230). A lost device ends the next step with `DeviceLost`, and what the failure kept shuts down. A frame that waits for a region and one torn by a refusal are not reached on null, whose control offers a loss and a stall of presents alone: `lactose-app`'s `tests/frame/states.rs` drives both over a scripted render side |
| The present side | `tests/present.rs`, `ClientPresent` alone on null with targets the test draws, at E1, E1, E2, E2, E1, E1 in turn: the swapchain resized at the first call and at each change of extent and at no other, and each image a slot held destroyed, which null refuses while a slot holds it, once its successor's read is reached |
| The count | `tests/count.rs`, `Waiting` over scripted display-time reports and timeline values: no source; a present still behind the GPU; a report that covers every earlier present; the count rising one a frame; a report of an id never made here |
| Criterion 1 | The binary's fixed-frame mode, 600 frames, on the M1 under MoltenVK and KosmicKrisp with the Khronos layer, and under MoltenVK with Metal API and Metal shader validation, each validator's canary in its own process; the four mutations of decision 5.1 each fail it, each at its first wrong pixel. Under both drivers the last frame has 1,044 pixels surely drawn, 195,304 surely clear and 260 within the margin of an edge, every one as the reference says. Run by hand under the GPU lock (`docs/verification.md`); the Windows laptop's run is owed |
