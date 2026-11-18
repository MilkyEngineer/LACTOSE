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
  earlier submits of its queue, their waits or their acquires: an acquire is held to the
  change rule alone, and every later use waits for itself. The stamps are of the whole
  resource, not of each subresource: a `Concurrent` image changed at one mip on one queue
  holds back a use of another mip on another.
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
  against queues held from running; the drops that wait, of the swapchain with a present
  pending and of the device's last handle; a drop that is not the last, which returns at
  once; and the drops made while a thread unwinds, none of which waits.

## Proof

Every rule null keeps has a committed test that fails without it. For each row a build
with that check removed, and nothing else changed, fails the tests the row names: the
mutation is at least as strong as the rule, the whole check rather than one of its inputs.
`conformance::` tests run every case under the five sets, `null::` and `wakes::` are in
`tests/`, and `tests::` is `src/tests.rs`. Where one rule is checked in two places, there is
a row for each place and a third that removes both.

| Rule | Check removed | Fails |
|---|---|---|
| `state`, by subresource | a use's subresource read as the first | `conformance::state_is_kept_for_each_mip_and_layer` |
| `state`, by subresource | a transition's subresource read as the range's first | `conformance::state_is_kept_for_each_mip_and_layer`, `copy_regions_overlapping_in_the_destination_are_refused` |
| `state` | a use in another state than the one it needs | `conformance::use_in_another_state_is_refused`, `declared_buffer_in_another_state_is_refused`, `declared_slot_not_in_sampled_read_is_refused`, `state_is_kept_for_each_mip_and_layer` |
| `state` | the index buffer's state | `conformance::use_in_another_state_is_refused` |
| `state` | indirect arguments' state, for a draw | `conformance::use_in_another_state_is_refused` |
| `state` | indirect arguments' state, for a dispatch | `conformance::use_in_another_state_is_refused` |
| `state` | a second write with no transition | `conformance::use_after_a_write_without_a_transition_is_refused` |
| `state` | a second write through a `ShaderWrite` reach | `conformance::use_after_a_write_without_a_transition_is_refused`, `submit_writing_under_a_host_access_is_refused` |
| `state` | a buffer a pass declares in two states | `conformance::buffer_declared_in_two_states_is_refused` |
| `state` | a present of an image not in `Present` | `conformance::present_of_an_image_not_in_present_is_refused` |
| `no-qfot` | a use on a queue that does not own the resource | `conformance::cross_queue_use_without_a_transfer_is_refused` |
| `no-qfot` | an acquire with no release | `conformance::acquire_without_a_release_is_refused`, `acquire_of_a_release_a_retake_ended_is_refused` |
| `no-qfot` | a use between a release and its acquire | `conformance::acquire_without_a_release_is_refused` |
| `no-qfot` | an image transition under another queue's pending use | `conformance::image_taken_while_another_queue_reads_it_is_refused` |
| `no-qfot` | a buffer transition under another queue's pending use | `conformance::image_taken_while_another_queue_reads_it_is_refused` |
| `no-qfot` | an alias call on another queue than the other images' | `conformance::aliased_image_used_without_the_alias_call_is_refused` |
| `no-qfot` | an alias call on another queue than its own image's | `conformance::aliased_image_used_without_the_alias_call_is_refused` |
| `no-qfot`, the use rule | at an image use | `conformance::use_under_another_queues_pending_write_is_refused` |
| `no-qfot`, the use rule | at a buffer use | `conformance::use_under_another_queues_pending_write_is_refused`, `use_in_a_later_submit_than_the_waiting_change_is_refused` |
| `no-qfot`, the use rule | at an image transition | `conformance::use_in_a_later_submit_than_the_waiting_change_is_refused` |
| `no-qfot`, the use rule | at a buffer transition | `conformance::use_in_a_later_submit_than_the_waiting_change_is_refused` |
| `no-qfot`, the use rule | the waits only, not the queue's earlier submits and theirs | `conformance::use_in_a_later_submit_than_the_waiting_change_is_refused` |
| `no-qfot`, the use rule | an image's change stamped at the commit | `conformance::use_under_another_queues_pending_write_is_refused`, `use_in_a_later_submit_than_the_waiting_change_is_refused` |
| `no-qfot`, the use rule | a copy into an image, an attachment's write, an alias call, each as a change | `conformance::use_in_a_later_submit_than_the_waiting_change_is_refused` (a row each) |
| `no-qfot`, the use rule | an acquire's order, of an image and of a buffer | `conformance::use_in_a_later_submit_than_the_waiting_change_is_refused` (a row each) |
| `present-queue` | a swapchain image on another encoder than the present queue's | `conformance::swapchain_image_on_a_frame_encoder_is_refused` |
| `destroyed` | a handle destroyed after it was recorded | `conformance::destroyed_after_recording_is_refused_at_submit` |
| `destroyed` | a stale handle's generation, so a reused index resolves | `null::a_handle_whose_index_another_buffer_took_resolves_nothing` |
| `destroyed` | a present's waited timeline destroyed meanwhile | `null::a_destroy_races_a_present_wait`, `tests::a_timeline_destroyed_during_a_present_wait_refuses_the_present`, `wakes::a_destroy_during_a_present_wait_refuses_it_at_once` |
| `pending` | a buffer destroyed under a pending submit | `conformance::destroy_after_an_unobserved_submit_is_refused`, `destroy_racing_a_submit_refuses_one`, `destroy_under_a_pending_submit_is_refused`, `concurrent_resource_destroyed_while_another_queue_uses_it_is_refused`, and others |
| `pending` | an image destroyed under a pending submit | `conformance::destroy_under_a_pending_submit_is_refused` |
| `pending` | a block destroyed with an image placed in it | `conformance::destroy_of_a_block_an_image_is_placed_in_is_refused` |
| `pending` | an encoder reset before its submit is seen | `conformance::encoder_reset_while_pending_is_refused`, `submit_is_pending_until_its_wait_is_reached` |
| `pending` | pending until seen, not until run | `conformance::destroy_after_an_unobserved_submit_is_refused` |
| `pending` | a host write under a pending writer | `conformance::write_under_a_pending_writer_is_refused`, `submit_racing_a_host_access_refuses_one` |
| `pending` | a host read under a pending writer or change | `conformance::read_under_a_pending_writer_is_refused`, `read_under_a_pending_transition_to_host_read_is_refused` |
| `pending` | an invalidate under a pending writer or change | `conformance::read_under_a_pending_writer_is_refused`, `read_under_a_pending_transition_to_host_read_is_refused`, `null::a_read_of_a_written_range_needs_an_invalidate_where_memory_is_not_coherent` |
| `pending` | a transition stamped as a change | `conformance::read_under_a_pending_transition_to_host_read_is_refused` |
| `pending` | a transition stamped as a write | `conformance::write_under_a_pending_writer_is_refused` |
| `pending` | a host read, or an invalidate, waiting for writes alone | `conformance::read_under_a_pending_transition_to_host_read_is_refused` (a row each) |
| `pending` | a host write waiting for changes too | `conformance::write_under_a_pending_writer_is_refused` |
| `pending` | a submit's writes stamped on the buffer | `conformance::write_under_a_pending_writer_is_refused`, `submit_racing_a_host_access_refuses_one` |
| `host-overlap` | a host write, or a host read, over a host access | `conformance::overlapping_host_access_is_refused` (a row each) |
| `host-overlap` | a destroy during a host access | `conformance::destroy_during_host_access_is_refused` |
| `host-overlap` | a submit that writes a buffer during a host access | `conformance::submit_writing_under_a_host_access_is_refused`, `submit_racing_a_host_access_refuses_one`, `tests::a_submit_that_writes_during_a_host_write_is_refused` |
| `uuwp-used` | a slot rewritten under a pending submit | `conformance::heap_write_under_a_pending_reader_is_refused`, `heap_write_racing_a_submit_refuses_one` |
| `uuwp-used` | a pass begun during a slot's write | `tests::a_pass_that_begins_during_a_heap_write_is_refused`, `conformance::heap_write_racing_a_submit_refuses_one` |
| `heap-layout` | a slot written since its pass began | `conformance::slot_written_after_its_pass_began_is_refused`, `heap_write_racing_a_submit_refuses_one` |
| `heap-layout` | a slot's image in another state than `SampledRead` | `conformance::declared_slot_not_in_sampled_read_is_refused` |
| `heap-layout` | a draw that declares an empty slot | `conformance::declared_slot_empty_is_refused` |
| `encoding` | an encoder of another device, or of another queue | `conformance::encoder_of_another_role_or_device_is_refused` (a row each) |
| `encoding` | an encoder submitted again | `conformance::encoder_in_two_submits_is_refused`, `encoder_resubmitted_without_reset_is_refused` |
| `encoding` | a pass left open | `conformance::pass_left_open_is_refused` |
| `encoding` | a draw outside a render pass | `conformance::command_outside_its_scope_is_refused` |
| `encoding` | a draw with no pipeline | `conformance::draw_with_nothing_bound_is_refused` |
| `usage` | a buffer used outside its usage | `conformance::buffer_use_outside_its_usage_is_refused` |
| `usage` | an image used outside its usage | `conformance::image_use_outside_its_usage_is_refused` |
| `usage` | a heap slot written with an image that is not `SAMPLED` | `conformance::image_use_outside_its_usage_is_refused` |
| `usage` | a transition into a state the image's usage does not allow | `conformance::image_use_outside_its_usage_is_refused` |
| `usage` | a pipeline of more colour targets than `colour_attachments` | `conformance::pipeline_of_other_formats_than_its_pass_is_refused` |
| `usage` | a copy from a buffer to itself | `null::the_refusals_null_alone_makes_name_their_rules` |
| `capability` | a module capability the device does not report | `conformance::shader_needing_an_unenabled_capability_is_refused` |
| `capability` | multi-draw indirect where it is not reported | `conformance::unreported_capability_is_refused` |
| `capability` | a count draw where it is not reported | `conformance::unreported_capability_is_refused` |
| `bounds` | a range past its resource | `conformance::copy_region_outside_its_resource_is_refused`, `host_range_outside_the_buffer_is_refused`, `indirect_arguments_outside_or_unaligned_are_refused`, `placement_outside_the_block_is_refused` |
| `bounds` | a copy rectangle outside its mip | `conformance::copy_region_outside_its_resource_is_refused` |
| `bounds` | a buffer-image copy's offset not a multiple of the texel | `conformance::copy_region_outside_its_resource_is_refused` |
| `bounds` | an address past its buffer, a copy region of no bytes | `null::the_refusals_null_alone_makes_name_their_rules` (a row each) |
| `bounds` | two regions of one copy that write one destination byte, of a buffer copy and of an image-to-buffer copy | `conformance::copy_regions_overlapping_in_the_destination_are_refused` (a row each) |
| `bounds` | two regions of one buffer-to-image copy that write one texel | `conformance::copy_regions_overlapping_in_the_destination_are_refused` |
| `bounds` | the overlap read in the order given, rectangles meeting by their columns alone, regions of two mips or layers meeting, every two rectangles meeting | `conformance::copy_regions_overlapping_in_the_destination_are_refused` (a row each) |
| `undefined-read` | a read of contents not defined | `conformance::alias_read_before_its_first_write_is_refused`, `read_of_contents_left_undefined_is_refused`, `read_after_a_retake_of_an_unmet_release_is_refused` |
| `undefined-read` | a transition from `Undefined` that keeps an image's contents | `conformance::read_of_contents_left_undefined_is_refused` |
| `undefined-read` | a `DontCare` store that defines contents | `conformance::read_of_contents_left_undefined_is_refused` |
| `undefined-read` | a buffer taken from `Undefined` onto another queue that keeps its contents | `conformance::read_of_contents_left_undefined_is_refused`, `read_after_a_retake_of_an_unmet_release_is_refused` |
| `undefined-read` | a re-take of an unmet release that keeps the bytes | `conformance::read_after_a_retake_of_an_unmet_release_is_refused` |
| `no-fence` | a placed image used after another with no alias call | `conformance::aliased_image_used_without_the_alias_call_is_refused` |
| `no-fence` | a placed image an alias call ended, used once the other is destroyed | `conformance::aliased_image_used_without_the_alias_call_is_refused` |
| `undeclared` | an address the pass did not declare | `conformance::undeclared_address_argument_is_refused` |
| `undeclared` | a slot the pass did not declare | `conformance::undeclared_slot_argument_is_refused` |
| `unflushed` | a written range not flushed at a submit | `conformance::unflushed_range_at_a_submit_is_refused` |
| `unflushed` | a host lend's range left unflushed | `conformance::unflushed_range_at_a_submit_is_refused` |
| `unflushed` | a read of a written range with no invalidate | `null::a_read_of_a_written_range_needs_an_invalidate_where_memory_is_not_coherent` |
| `local-map` | a local read mapped to another attachment | `conformance::input_read_of_another_attachment_is_refused` |
| `local` | an input read with no local read | `conformance::input_read_without_a_local_read_is_refused` |
| `split` | a memoryless attachment after a local read, where lost | `conformance::memoryless_attachment_after_a_local_read_is_refused_where_lost` |
| `split` | the depth attachment's check alone | `conformance::memoryless_attachment_after_a_local_read_is_refused_where_lost` |
| `memoryless-store` | a memoryless attachment kept or stored | `conformance::memoryless_attachment_kept_is_refused`, `memoryless_attachment_stored_is_refused` |
| `swapchain` | a second swapchain | `conformance::second_swapchain_is_refused` |
| `swapchain` | an acquire with one held | `conformance::second_acquire_before_a_present_is_refused`, `present_behind_an_unmet_wait_is_refused`, and others |
| `swapchain` | a present of an image not acquired | `conformance::present_of_an_image_not_acquired_is_refused` |
| `swapchain` | a destroy of its image | `null::the_refusals_null_alone_makes_name_their_rules` |
| `swapchain` | a use outside its acquire | `conformance::swapchain_image_used_outside_its_acquire_is_refused` |
| `swapchain` | a transition outside its acquire | `conformance::swapchain_image_used_outside_its_acquire_is_refused` |
| `swapchain` | both of them | `conformance::swapchain_image_used_outside_its_acquire_is_refused` |
| `timeline` | a host signal not below a pending promise | `conformance::host_signal_above_a_pending_signal_is_refused` |
| `timeline` | a timeline signalled from two queues with no wait between | `conformance::timeline_signalled_from_two_queues_without_a_wait_is_refused` |
| `timeline` | a signal that does not wait for a host signal in progress | `tests::a_host_signal_between_another_host_signals_holds_is_refused` |
| `timeline` | two host signals at once | `tests::a_host_signal_between_another_host_signals_holds_is_refused`, `a_host_signal_landed_between_another_host_signals_holds_keeps_the_timeline_growing` |
| `timeline`, the 2^31 limit | a difference of exactly 2^31 | `conformance::timeline_value_beyond_the_difference_limit_is_refused`, `tests::a_wait_only_submit_2_31_below_a_marked_host_signal_is_refused` |
| `timeline`, the 2^31 limit | the marked host signal left out | `tests::a_wait_only_submit_2_31_below_a_marked_host_signal_is_refused` |
| `timeline`, the 2^31 limit | pending waits and signals left out | `conformance::timeline_value_beyond_the_difference_limit_is_refused` |
| `timeline`, the 2^31 limit | the timeline's value left out | `conformance::timeline_value_beyond_the_difference_limit_is_refused` |
| The present's walk | the greatest promise taken, not the least | `tests::the_least_promise_at_or_above_a_point_decides_it`, `a_decided_submit_is_not_taken_for_one_in_progress` |
| The present's walk | a cycle taken as decided | `tests::a_cycle_across_two_queues_is_undecided`, `a_walk_back_to_a_submit_in_progress_leaves_the_point_undecided`, `conformance::present_behind_an_unmet_wait_is_refused` |
| The present's walk | a submit in progress taken as decided | `tests::a_cycle_across_two_queues_is_undecided`, `a_walk_back_to_a_submit_in_progress_leaves_the_point_undecided`, `conformance::present_behind_an_unmet_wait_is_refused` |
| The present's walk | a decided submit taken as in progress | `tests::a_decided_submit_is_not_taken_for_one_in_progress` |
| The stall | presents run while stalled | `conformance::resize_with_a_present_pending_is_refused` |
| Loss | the injected loss not kept | `conformance::loss_is_returned_and_stays`, `waits_return_at_once_after_a_loss`, `wait_is_woken_by_a_loss`, `teardown_succeeds_after_a_loss`, `null::a_loss_reaches_every_call_and_the_control_outlives_the_device`, and others |
| The last drop | a device's last drop that does not wait for its submits | `tests::the_last_drop_of_a_device_waits_for_its_pending_submits` |
| The cooked format | `encode` without the checks `decode` makes | `conformance::cooked_shader_round_trips` |

The threads' rules, each removed alone in the same way:

| What is removed | Fails |
|---|---|
| a present's wait releases its queue's lock | `null::submits_on_two_queues_race_a_present`, `wakes::a_present_waiting_on_a_shared_queue_lets_the_frame_submit_in` |
| the wake after a submit | `conformance::waits_are_woken_by_the_submit_that_reaches_them` |
| the wake after a host signal | `conformance::wait_is_woken_by_a_host_signal`, `destroy_at_once_after_the_wait_is_accepted`, `pending_encoder_drop_waits_for_its_submit` |
| the wake after a timeline's destroy | `wakes::a_destroy_wakes_a_wait`, `a_destroy_during_a_present_wait_refuses_it_at_once` |
| the wake after a loss | `conformance::wait_is_woken_by_a_loss`, `wakes::a_loss_during_a_present_wait_returns_lost`, `tests::the_last_drop_of_a_device_waits_for_its_pending_submits` |
| the wake after a stall ends | `wakes::an_unstall_wakes_an_acquire` |
| the wake after a present | `wakes::a_present_wakes_a_wait` |
| the heap's lock over a slot's write | `tests::a_heap_write_started_inside_another_waits_for_it` |
| a present's wait seeing a loss | `wakes::a_loss_during_a_present_wait_returns_lost` |
| a pending encoder's drop waiting for its submit | `conformance::pending_encoder_drop_waits_for_its_submit` |
| a swapchain's drop waiting for its present's submit | `tests::a_swapchain_drop_with_its_present_pending_waits_for_its_submit` |
| generations shared by every null device | `conformance::handle_of_another_device_is_refused` |
| every waiter woken, not one | `conformance::waits_are_woken_by_the_submit_that_reaches_them` |

`lost`'s store made `Relaxed` fails nothing, and is no defect: `is_lost` reads only the
flag, and every call that acts on a loss reads the records' own flag under their lock.
