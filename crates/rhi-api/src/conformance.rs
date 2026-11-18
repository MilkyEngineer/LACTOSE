//! The conformance suite's shape: its cases by name, each a contract case or an output
//! case, and the harness a backend gives it. Behind the `conformance` feature, for
//! `[dev-dependencies]` alone.
//!
//! A *contract* case asserts what the contract says of calls and their results: what is
//! accepted, what is refused and by which rule, what a query returns. It needs no pixel
//! and no shader to run, so the null backend passes it as a device does. An *output* case
//! asserts what a device produced: it draws, dispatches or copies, reads the result back
//! and compares it with a reference computed on the CPU, never with a stored image. Only
//! a device can pass it, and [`run`] skips it for a harness that does not render.
//!
//! The contract cases' bodies are here, one function for each, by group, written with
//! `lactose-rhi-null`; the output cases' and their shader sources arrive with
//! `lactose-rhi-vulkan` (milestone 2 plan, section 3). `README.md`, "The conformance
//! suite", says what each case holds.

mod buffers;
mod encoders;
mod heap;
mod images;
mod kit;
mod passes;
mod queues;
mod shaders;
mod states;
mod swapchain;
mod timelines;

use std::sync::Arc;

use lactose_core::Result;

use crate::caps::Capabilities;
use crate::device::Device;
use crate::message::MessageSink;
use crate::rule::Rule;
use crate::shader::{CookedShader, ShaderStage};

/// Which backends can pass a case.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaseKind {
    /// Every backend, the null one included.
    Contract,
    /// A backend that renders.
    Output,
}

/// One case of the suite.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Case {
    /// The case's name, unique in [`CASES`]. A backend's test for the case carries it.
    pub name: &'static str,
    /// Which backends can pass it.
    pub kind: CaseKind,
    /// The rule the case exists to see refused, for a case that provokes a refusal: the
    /// case fails unless the call is refused by exactly this rule, so a backend built
    /// without the rule fails it.
    pub refuses: Option<Rule>,
    /// Whether a device backend runs the case. `false` for a case that provokes a check
    /// the null backend alone makes, whose breach on a device gives a wrong image and
    /// proves nothing, and for one that holds null's own meaning of a pending submit, one
    /// whose completion the caller has not yet seen (`README.md`, "Queues by role").
    pub devices: bool,
}

/// What running a case came to. A case that fails panics, as a test does.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Outcome {
    /// The case ran under at least one capability set and held under each.
    Passed,
    /// The case did not run, for the reason given: an output case on a harness that does
    /// not render, a case that is not for devices ([`Case::devices`]) on one that does, or
    /// a case that needs a capability or a window no set of the harness has. A null
    /// harness that skips a contract case is incomplete.
    Skipped(&'static str),
}

/// What a backend gives the suite.
pub trait Harness {
    /// The capability sets the backend runs the suite under. The null backend names the
    /// sets its contract needs: one queue and several, coherent memory and not,
    /// memoryless contents kept across a local read and lost, each optional capability
    /// on and off. A device backend names its device's own.
    fn sets(&self) -> Vec<Capabilities>;

    /// A new device that reports exactly `set`, one of [`sets`](Self::sets), with
    /// `messages` as its sink. With `presents` it has a surface to make a swapchain for:
    /// null's is its own, and needs no window.
    ///
    /// # Errors
    ///
    /// What the backend's constructor returns; `ErrorKind::Unsupported` for `presents`
    /// where the run has no window, and the swapchain's cases are then skipped.
    fn device(
        &self,
        set: &Capabilities,
        messages: Arc<dyn MessageSink>,
        presents: bool,
    ) -> Result<Box<dyn Device>>;

    /// A new device as [`device`](Self::device) makes with no surface, whose request's
    /// `pipeline_cache` is `cache`: the bytes another device's `pipeline_cache` returned,
    /// or bytes that fail its checks.
    ///
    /// # Errors
    ///
    /// What the backend's constructor returns, which bytes that fail a check never make it
    /// return.
    fn device_with_cache(
        &self,
        set: &Capabilities,
        messages: Arc<dyn MessageSink>,
        cache: Vec<u8>,
    ) -> Result<Box<dyn Device>>;

    /// Whether the backend's devices produce output. `false` for null, and [`run`] then
    /// skips every output case.
    fn renders(&self) -> bool;

    /// The suite's cooked shader of that name and stage, for the target the backend
    /// runs. Called by output cases alone.
    ///
    /// # Errors
    ///
    /// `ErrorKind::NotFound` when the shaders were not cooked before the run.
    fn shader(&self, name: &str, stage: ShaderStage) -> Result<CookedShader>;

    /// Makes `device` lost, as a driver would, and returns `true`; or returns `false`
    /// where the backend cannot, and the cases that need a loss are skipped. Null can.
    fn lose(&self, device: &dyn Device) -> bool;

    /// With `on`, keeps `device`'s presents from completing, as a display that has not taken
    /// them would: their submits complete, and the driver's read of each image does not end
    /// until it is called again with `on` false; returns `true`. Returns `false` where the
    /// backend does not offer it, which no device does, and a case that needs a stall then
    /// takes its device half. Null offers it. A device dropped while stalled is unstalled
    /// first.
    fn stall(&self, device: &dyn Device, on: bool) -> bool;
}

/// Runs one case of [`CASES`] against a backend, under every capability set of the
/// harness the case applies to. Each device it makes has a sink that counts, and the case
/// fails on any message it did not provoke.
///
/// # Panics
///
/// When the case fails, with the case's name and what did not hold.
pub fn run(harness: &dyn Harness, case: &Case) -> Outcome {
    if case.kind == CaseKind::Output {
        return Outcome::Skipped(if harness.renders() {
            "an output case, whose body arrives with lactose-rhi-vulkan"
        } else {
            "an output case, on a harness that does not render"
        });
    }
    if harness.renders() && !case.devices {
        return Outcome::Skipped("a case of a check the null backend alone makes");
    }
    let Some(body) = body(case.name) else { panic!("{}: a contract case with no body", case.name) };
    let mut ran = false;
    let mut skipped = "no capability set of the harness";
    for set in harness.sets() {
        let ctx = kit::Ctx::new(harness, case, set);
        match body(&ctx) {
            Ok(()) => ran = true,
            Err(why) => skipped = why,
        }
        let messages = ctx.messages();
        assert!(
            messages == 0,
            "{} under {set:?}: {messages} messages a clean run does not send",
            case.name
        );
    }
    if ran { Outcome::Passed } else { Outcome::Skipped(skipped) }
}

/// Every contract case's body, by the case's name.
fn bodies() -> impl Iterator<Item = (&'static str, kit::Body)> {
    [
        queues::BODIES,
        buffers::BODIES,
        images::BODIES,
        states::BODIES,
        heap::BODIES,
        shaders::BODIES,
        encoders::BODIES,
        timelines::BODIES,
        passes::BODIES,
        swapchain::BODIES,
    ]
    .into_iter()
    .flatten()
    .copied()
}

fn body(name: &str) -> Option<kit::Body> {
    bodies().find(|(case, _)| *case == name).map(|(_, body)| body)
}

/// Declares the case table: `contract "name";`, `output "name";`,
/// `contract "name" => Rule;` for a refusal the null backend alone makes, and
/// `contract "name" => Rule everywhere;` for one every backend makes.
macro_rules! cases {
    ($( $kind:ident $name:literal $(=> $rule:ident $($all:ident)?)? ; )+) => {
        &[ $( Case {
            name: $name,
            kind: cases!(@kind $kind),
            refuses: cases!(@rule $($rule)?),
            devices: cases!(@devices $($rule $($all)?)?),
        } ),+ ]
    };
    (@kind contract) => { CaseKind::Contract };
    (@kind output) => { CaseKind::Output };
    (@rule) => { None };
    (@rule $rule:ident) => { Some(Rule::$rule) };
    (@devices) => { true };
    (@devices $rule:ident) => { false };
    (@devices $rule:ident everywhere) => { true };
}

/// Every case of the suite. `README.md`, "The conformance suite", states each.
pub const CASES: &[Case] = cases! {
    // Capabilities and queues.
    contract "capabilities_are_fixed";
    contract "unreported_capability_is_refused" => Capability everywhere;
    contract "roles_resolve_as_the_layout_reports";
    contract "submit_to_the_present_role_is_refused" => PresentQueue everywhere;
    // Buffers and host access.
    contract "buffer_write_then_copy_then_read_returns_the_bytes";
    contract "buffer_address_is_stable_and_distinct";
    contract "host_ranges_that_do_not_overlap_are_written_at_once";
    contract "overlapping_host_access_is_refused" => HostOverlap everywhere;
    contract "destroy_during_host_access_is_refused" => HostOverlap everywhere;
    contract "host_range_outside_the_buffer_is_refused" => Bounds everywhere;
    contract "host_access_outside_the_memory_class_is_refused" => Usage everywhere;
    contract "address_of_a_buffer_without_the_usage_is_refused" => Usage everywhere;
    contract "read_under_a_pending_writer_is_refused" => Pending everywhere;
    contract "read_under_a_pending_transition_to_host_read_is_refused" => Pending everywhere;
    contract "write_under_a_pending_writer_is_refused" => Pending everywhere;
    contract "submit_writing_under_a_host_access_is_refused" => HostOverlap everywhere;
    contract "submit_racing_a_host_access_refuses_one";
    contract "unflushed_range_at_a_submit_is_refused" => Unflushed;
    contract "flush_and_invalidate_are_accepted_where_memory_is_coherent";
    contract "buffer_use_outside_its_usage_is_refused" => Usage everywhere;
    output "device_memory_is_filled_through_a_staging_copy";
    // Images, memoryless requests and aliased transients.
    contract "image_use_outside_its_usage_is_refused" => Usage everywhere;
    contract "memoryless_outcome_is_queryable_and_as_reported";
    contract "memoryless_request_without_transient_is_refused" => Usage everywhere;
    contract "memoryless_attachment_stored_is_refused" => MemorylessStore everywhere;
    contract "memoryless_attachment_kept_is_refused" => MemorylessStore everywhere;
    contract "memoryless_attachment_after_a_local_read_is_refused_where_lost" => Split;
    contract "placement_outside_the_block_is_refused" => Bounds everywhere;
    contract "destroy_of_a_block_an_image_is_placed_in_is_refused" => Pending everywhere;
    contract "aliased_image_used_without_the_alias_call_is_refused" => Alias;
    contract "alias_read_before_its_first_write_is_refused" => UndefinedRead;
    contract "aliases_used_in_turn_with_the_alias_call_pass";
    output "memoryless_g_buffer_is_lit_in_its_pass";
    output "aliased_transients_render_in_turn";
    output "image_round_trips_through_copies";
    // States and queues.
    contract "transition_from_another_state_is_refused" => State;
    contract "use_in_another_state_is_refused" => State;
    contract "use_after_a_write_without_a_transition_is_refused" => State;
    contract "state_is_kept_for_each_mip_and_layer";
    contract "state_follows_submission_order_not_recording_order";
    contract "cross_queue_use_without_a_transfer_is_refused" => Queue;
    contract "acquire_without_a_release_is_refused" => Queue;
    contract "release_and_acquire_cross_a_resource";
    contract "shared_and_discarded_resources_cross_without_a_transfer";
    contract "roles_on_one_queue_need_no_transfer";
    contract "image_taken_while_another_queue_reads_it_is_refused" => Queue;
    contract "read_of_contents_left_undefined_is_refused" => UndefinedRead;
    contract "use_under_another_queues_pending_write_is_refused" => Queue;
    contract "use_under_another_queues_pending_transition_is_refused" => Queue;
    contract "use_in_a_later_submit_than_the_waiting_change_is_refused" => Queue;
    contract "take_of_a_mip_after_an_acquire_of_another_is_refused" => Queue;
    contract "use_waiting_through_another_submit_passes";
    contract "acquire_of_a_release_a_retake_ended_is_refused" => Queue;
    contract "read_after_a_retake_of_an_unmet_release_is_refused" => UndefinedRead;
    output "ownership_transfer_keeps_the_contents";
    // The heap, the sampler table and what a pass declares.
    contract "heap_slot_outside_the_heap_is_refused" => Bounds everywhere;
    contract "declared_slot_empty_is_refused" => HeapLayout everywhere;
    contract "slot_written_after_its_pass_began_is_refused" => HeapLayout everywhere;
    contract "declared_slot_not_in_sampled_read_is_refused" => HeapLayout;
    contract "heap_write_under_a_pending_reader_is_refused" => HeapWrite everywhere;
    contract "heap_write_after_its_reader_completes_passes";
    contract "heap_write_racing_a_submit_refuses_one";
    contract "destroy_of_an_image_a_slot_holds_is_refused" => Pending everywhere;
    contract "sampler_destroyed_under_a_pending_submit_is_refused" => Pending everywhere;
    contract "undeclared_slot_argument_is_refused" => Undeclared everywhere;
    contract "undeclared_address_argument_is_refused" => Undeclared everywhere;
    contract "declared_buffer_in_another_state_is_refused" => State;
    contract "buffer_declared_in_two_states_is_refused" => State everywhere;
    contract "sampler_table_is_bounded";
    output "heap_is_sampled_past_slot_zero";
    output "address_is_read_in_a_shader";
    output "declared_reader_follows_a_lagging_writer";
    output "sampler_filters_as_described";
    // Cooked shaders, pipelines and the cache.
    contract "cooked_shader_round_trips";
    contract "cooked_shader_of_another_version_is_refused";
    contract "cooked_shader_corrupt_or_truncated_is_refused";
    contract "shader_for_another_heap_is_refused";
    contract "shader_target_the_backend_does_not_run_is_unsupported";
    contract "constant_of_another_length_is_refused";
    contract "shader_needing_an_unenabled_capability_is_refused" => Capability everywhere;
    contract "pipeline_of_other_formats_than_its_pass_is_refused" => Usage everywhere;
    contract "sealed_cache_round_trips";
    contract "cache_corrupt_truncated_or_foreign_starts_empty";
    output "specialised_constant_changes_the_output";
    output "write_mask_keeps_a_channel";
    output "depth_state_orders_two_triangles";
    output "warm_cache_hits_every_stage";
    // Encoders, submits and timelines.
    contract "encoder_recorded_on_another_thread_submits";
    contract "encoder_reset_while_pending_is_refused" => Pending everywhere;
    contract "pending_encoder_drop_waits_for_its_submit";
    contract "encoder_in_two_submits_is_refused" => Encoding everywhere;
    contract "encoder_resubmitted_without_reset_is_refused" => Encoding everywhere;
    contract "recording_while_pending_records_nothing";
    contract "encoder_of_another_role_or_device_is_refused" => Encoding everywhere;
    contract "command_outside_its_scope_is_refused" => Encoding everywhere;
    contract "draw_with_nothing_bound_is_refused" => Encoding everywhere;
    contract "pass_left_open_is_refused" => Encoding everywhere;
    contract "push_outside_the_block_is_refused" => Bounds everywhere;
    contract "copy_region_outside_its_resource_is_refused" => Bounds everywhere;
    contract "copy_regions_overlapping_in_the_destination_are_refused" => Bounds everywhere;
    contract "refused_submit_changes_nothing";
    contract "submit_of_no_encoders_waits_and_signals";
    contract "submit_is_pending_until_its_wait_is_reached";
    contract "empty_draws_and_dispatches_record_nothing";
    contract "dispatch_beyond_the_group_limit_is_refused" => Bounds everywhere;
    contract "timeline_is_signalled_waited_and_read_by_the_host";
    contract "timeline_wait_times_out";
    contract "waits_are_woken_by_the_submit_that_reaches_them";
    contract "wait_is_woken_by_a_host_signal";
    contract "timeline_value_that_does_not_grow_is_refused" => Timeline everywhere;
    contract "host_signal_above_a_pending_signal_is_refused" => Timeline everywhere;
    contract "host_signal_below_a_promise_that_does_not_wait_for_it_is_refused" => Timeline everywhere;
    contract "host_signal_below_a_pending_signal_releases_its_waiter";
    contract "host_signal_below_a_promise_whose_wait_is_met_is_refused" => Timeline everywhere;
    contract "submit_signalling_no_more_than_it_waits_for_is_refused" => Timeline everywhere;
    contract "timeline_value_beyond_the_difference_limit_is_refused" => Timeline everywhere;
    contract "timeline_signalled_from_two_queues_without_a_wait_is_refused" => Timeline everywhere;
    contract "destroy_under_a_pending_submit_is_refused" => Pending everywhere;
    contract "concurrent_resource_destroyed_while_another_queue_uses_it_is_refused" => Pending everywhere;
    contract "destroy_after_an_unobserved_submit_is_refused" => Pending;
    contract "destroy_at_once_after_the_wait_is_accepted";
    contract "destroy_racing_a_submit_refuses_one";
    contract "submits_racing_on_one_queue_keep_their_order";
    contract "host_signals_racing_on_one_timeline_keep_it_growing";
    contract "use_after_destruction_is_refused" => Destroyed everywhere;
    contract "destroyed_after_recording_is_refused_at_submit" => Destroyed everywhere;
    contract "handle_of_another_device_is_refused" => Destroyed;
    output "two_threads_record_at_once";
    // Render passes and the local read.
    contract "input_read_without_a_local_read_is_refused" => Local;
    contract "input_read_of_another_attachment_is_refused" => LocalMap;
    contract "attachments_of_unequal_extent_are_refused" => Usage everywhere;
    contract "scissor_outside_the_attachments_is_refused" => Bounds everywhere;
    contract "viewport_outside_its_rules_is_refused" => Bounds everywhere;
    output "triangle_matches_the_reference";
    output "local_read_sees_what_the_pass_wrote";
    output "clear_and_keep_load_as_described";
    output "viewport_and_scissor_place_the_triangles";
    output "draw_parameters_reach_the_shader";
    // Indirect draws and dispatches.
    contract "index_buffer_offset_outside_or_unaligned_is_refused" => Bounds everywhere;
    contract "indexed_draw_past_its_index_buffer_is_refused" => Bounds everywhere;
    contract "indirect_arguments_outside_or_unaligned_are_refused" => Bounds everywhere;
    output "indexed_indirect_draw_matches_the_reference";
    output "indirect_count_draws_where_reported";
    output "dispatch_writes_through_an_address";
    output "dispatches_of_one_pass_see_each_others_writes";
    output "dispatch_indirect_reads_its_counts";
    // The swapchain and the present role.
    contract "second_acquire_before_a_present_is_refused" => Swapchain everywhere;
    contract "present_of_an_image_not_acquired_is_refused" => Swapchain everywhere;
    contract "present_behind_an_unmet_wait_is_refused" => Pending everywhere;
    contract "present_of_an_image_not_in_present_is_refused" => State;
    contract "swapchain_image_on_a_frame_encoder_is_refused" => PresentQueue;
    contract "swapchain_image_used_outside_its_acquire_is_refused" => Swapchain;
    contract "second_swapchain_is_refused" => Swapchain everywhere;
    contract "swapchain_extent_with_a_zero_dimension_is_refused" => Bounds everywhere;
    contract "resize_with_a_present_pending_is_refused" => Pending everywhere;
    contract "image_of_before_a_resize_is_refused" => Destroyed everywhere;
    contract "frame_submits_complete_while_a_present_is_pending";
    // Device loss and messages.
    contract "loss_is_returned_and_stays";
    contract "waits_return_at_once_after_a_loss";
    contract "wait_is_woken_by_a_loss";
    contract "teardown_succeeds_after_a_loss";
    contract "clean_run_sends_no_message";
    output "each_validator_reports_its_canary";
};

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{CASES, CaseKind, bodies};

    #[test]
    fn every_contract_case_has_one_body_and_every_body_one_case() {
        let mut named = HashSet::new();
        for (name, _) in bodies() {
            assert!(named.insert(name), "{name}: two bodies");
            assert!(
                CASES.iter().any(|case| case.name == name && case.kind == CaseKind::Contract),
                "{name}: a body for no contract case"
            );
        }
        for case in CASES.iter().filter(|case| case.kind == CaseKind::Contract) {
            assert!(named.contains(case.name), "{}: a contract case with no body", case.name);
        }
    }
}
