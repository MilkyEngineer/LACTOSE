//! The conformance suite of `lactose-rhi-api` against the null backend: one test for each
//! case, which runs it under every capability set of [`Null`], and a check that the tests
//! are the suite's cases. A contract case passes; an output case is skipped, since null
//! draws nothing.

use std::sync::{Arc, Mutex, PoisonError};

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::conformance::{self, CASES, Case, CaseKind, Harness, Outcome};
use lactose_rhi_api::{
    Capabilities, CookedShader, Device, DeviceRequest, MessageSink, QueueLayout, QueuePath,
    ShaderStage,
};
use lactose_rhi_null::{Control, NullDevice};

use crate::{Silent, request};

/// The harness: five capability sets, and the control of each device it made.
#[derive(Default)]
struct Null {
    /// Each device's control, by the address of the device while it lives.
    controls: Mutex<Vec<(usize, Control)>>,
}

/// The capability sets the contract needs, between them: one queue and several, each role
/// on a queue of its own and on the graphics queue, coherent memory and not, memoryless
/// contents kept across a local read and lost, each optional capability on and off, and
/// MoltenVK 1.4.2's shape on the M1 (the RHI spike): multi-draw indirect with no count draw,
/// and memoryless depth alone, lost across a local read.
pub(crate) const SETS: [Capabilities; 5] = [
    Capabilities::LEAST,
    Capabilities {
        multi_draw_indirect: true,
        indirect_count: true,
        buffer_atomics_64: true,
        memoryless_colour: true,
        memoryless_depth: true,
        memoryless_survives_local_read: true,
        coherent_host_memory: false,
        queues: QueueLayout::SEVERAL,
        ..Capabilities::LEAST
    },
    Capabilities {
        memoryless_colour: true,
        queues: QueueLayout { compute: QueuePath::Own, present: QueuePath::Shared },
        ..Capabilities::LEAST
    },
    Capabilities {
        multi_draw_indirect: true,
        buffer_atomics_64: true,
        coherent_host_memory: false,
        queues: QueueLayout { compute: QueuePath::Shared, present: QueuePath::Own },
        ..Capabilities::LEAST
    },
    Capabilities { multi_draw_indirect: true, memoryless_depth: true, ..Capabilities::LEAST },
];

/// The address a device is known by while it lives.
fn address(device: &dyn Device) -> usize {
    core::ptr::from_ref(device).cast::<()>() as usize
}

impl Null {
    fn control(&self, device: &dyn Device) -> Option<Control> {
        let controls = self.controls.lock().unwrap_or_else(PoisonError::into_inner);
        let at = address(device);
        controls.iter().find(|(known, _)| *known == at).map(|(_, control)| control.clone())
    }
}

impl Harness for Null {
    fn sets(&self) -> Vec<Capabilities> {
        SETS.to_vec()
    }

    fn device(
        &self,
        set: &Capabilities,
        messages: Arc<dyn MessageSink>,
        presents: bool,
    ) -> Result<Box<dyn Device>> {
        let device = NullDevice::new(request(set, presents, messages), *set)?;
        let control = device.control();
        let device: Box<dyn Device> = Box::new(device);
        let at = address(&*device);
        let mut controls = self.controls.lock().unwrap_or_else(PoisonError::into_inner);
        // A device that has gone may have left its address to this one.
        controls.retain(|(known, _)| *known != at);
        controls.push((at, control));
        Ok(device)
    }

    fn device_with_cache(
        &self,
        set: &Capabilities,
        messages: Arc<dyn MessageSink>,
        cache: Vec<u8>,
    ) -> Result<Box<dyn Device>> {
        let request = DeviceRequest { pipeline_cache: cache, ..request(set, false, messages) };
        Ok(Box::new(NullDevice::new(request, *set)?))
    }

    fn renders(&self) -> bool {
        false
    }

    fn shader(&self, name: &str, _: ShaderStage) -> Result<CookedShader> {
        Err(Error::new(
            ErrorKind::NotFound,
            format!("null runs no output case and has no `{name}`"),
        ))
    }

    fn lose(&self, device: &dyn Device) -> bool {
        self.control(device).is_some_and(|control| control.lose())
    }

    fn stall(&self, device: &dyn Device, on: bool) -> bool {
        self.control(device).is_some_and(|control| control.stall(on))
    }
}

fn case(name: &str) -> &'static Case {
    CASES.iter().find(|case| case.name == name).unwrap_or_else(|| panic!("no case `{name}`"))
}

/// Runs a contract case, which passes under at least one set.
fn contract(name: &str) {
    let case = case(name);
    assert_eq!(case.kind, CaseKind::Contract, "{name} is not a contract case");
    assert_eq!(
        conformance::run(&Null::default(), case),
        Outcome::Passed,
        "{name} ran under no set"
    );
}

/// Runs an output case, which null skips.
fn output(name: &str) {
    let case = case(name);
    assert_eq!(case.kind, CaseKind::Output, "{name} is not an output case");
    let outcome = conformance::run(&Null::default(), case);
    assert!(matches!(outcome, Outcome::Skipped(_)), "{name} ran on null: {outcome:?}");
}

/// One test for each case named, which runs it by `$check`, and the list of their names.
macro_rules! cases {
    ($list:ident, $check:ident: $($name:ident)+) => {
        const $list: &[&str] = &[$(stringify!($name)),+];
        $(
            #[test]
            fn $name() {
                $check(stringify!($name));
            }
        )+
    };
}

#[test]
fn every_case_of_the_suite_has_one_test() {
    let mut tested: Vec<&str> = CONTRACT.iter().chain(OUTPUT).copied().collect();
    let mut cases: Vec<&str> = CASES.iter().map(|case| case.name).collect();
    tested.sort_unstable();
    cases.sort_unstable();
    assert_eq!(tested, cases);
}

#[test]
fn the_sets_are_what_a_null_device_reports() {
    for set in SETS {
        let device =
            NullDevice::new(request(&set, false, Arc::new(Silent)), set).expect("a null device");
        assert_eq!(*device.capabilities(), set);
    }
}

cases! {
    CONTRACT, contract:
    capabilities_are_fixed
    unreported_capability_is_refused
    buffer_declared_in_two_states_is_refused
    read_of_contents_left_undefined_is_refused
    waits_are_woken_by_the_submit_that_reaches_them
    wait_is_woken_by_a_host_signal
    wait_is_woken_by_a_loss
    pending_encoder_drop_waits_for_its_submit
    roles_resolve_as_the_layout_reports
    submit_to_the_present_role_is_refused
    buffer_write_then_copy_then_read_returns_the_bytes
    device_buffer_written_by_a_pass_is_read_after_it
    buffer_address_is_stable_and_distinct
    host_ranges_that_do_not_overlap_are_written_at_once
    overlapping_host_access_is_refused
    destroy_during_host_access_is_refused
    host_range_outside_the_buffer_is_refused
    host_access_outside_the_memory_class_is_refused
    address_of_a_buffer_without_the_usage_is_refused
    read_under_a_pending_writer_is_refused
    read_under_a_pending_transition_to_host_read_is_refused
    write_under_a_pending_writer_is_refused
    submit_writing_under_a_host_access_is_refused
    submit_racing_a_host_access_refuses_one
    unflushed_range_at_a_submit_is_refused
    flush_and_invalidate_are_accepted_where_memory_is_coherent
    buffer_use_outside_its_usage_is_refused
    image_use_outside_its_usage_is_refused
    memoryless_outcome_is_queryable_and_as_reported
    memoryless_request_without_transient_is_refused
    memoryless_attachment_stored_is_refused
    memoryless_attachment_kept_is_refused
    memoryless_attachment_after_a_local_read_is_refused_where_lost
    placement_outside_the_block_is_refused
    destroy_of_a_block_an_image_is_placed_in_is_refused
    aliased_image_used_without_the_alias_call_is_refused
    alias_read_before_its_first_write_is_refused
    aliases_used_in_turn_with_the_alias_call_pass
    transition_from_another_state_is_refused
    use_in_another_state_is_refused
    use_after_a_write_without_a_transition_is_refused
    state_is_kept_for_each_mip_and_layer
    state_follows_submission_order_not_recording_order
    cross_queue_use_without_a_transfer_is_refused
    acquire_without_a_release_is_refused
    release_and_acquire_cross_a_resource
    shared_and_discarded_resources_cross_without_a_transfer
    roles_on_one_queue_need_no_transfer
    image_taken_while_another_queue_reads_it_is_refused
    use_under_another_queues_pending_write_is_refused
    use_under_another_queues_pending_transition_is_refused
    use_in_a_later_submit_than_the_waiting_change_is_refused
    take_of_a_mip_after_an_acquire_of_another_is_refused
    use_waiting_through_another_submit_passes
    acquire_of_a_release_a_retake_ended_is_refused
    read_after_a_retake_of_an_unmet_release_is_refused
    heap_slot_outside_the_heap_is_refused
    declared_slot_empty_is_refused
    slot_written_after_its_pass_began_is_refused
    declared_slot_not_in_sampled_read_is_refused
    heap_write_under_a_pending_reader_is_refused
    heap_write_after_its_reader_completes_passes
    heap_write_racing_a_submit_refuses_one
    destroy_of_an_image_a_slot_holds_is_refused
    sampler_destroyed_under_a_pending_submit_is_refused
    undeclared_slot_argument_is_refused
    undeclared_address_argument_is_refused
    declared_buffer_in_another_state_is_refused
    sampler_table_is_bounded
    cooked_shader_round_trips
    cooked_shader_of_another_version_is_refused
    cooked_shader_corrupt_or_truncated_is_refused
    shader_for_another_heap_is_refused
    shader_target_the_backend_does_not_run_is_unsupported
    constant_of_another_length_is_refused
    shader_needing_an_unenabled_capability_is_refused
    pipeline_of_other_formats_than_its_pass_is_refused
    sealed_cache_round_trips
    cache_corrupt_truncated_or_foreign_starts_empty
    encoder_recorded_on_another_thread_submits
    encoder_reset_while_pending_is_refused
    encoder_in_two_submits_is_refused
    encoder_resubmitted_without_reset_is_refused
    recording_while_pending_records_nothing
    encoder_of_another_role_or_device_is_refused
    command_outside_its_scope_is_refused
    draw_with_nothing_bound_is_refused
    pass_left_open_is_refused
    push_outside_the_block_is_refused
    copy_region_outside_its_resource_is_refused
    copy_regions_overlapping_in_the_destination_are_refused
    refused_submit_changes_nothing
    submit_of_no_encoders_waits_and_signals
    submit_is_pending_until_its_wait_is_reached
    empty_draws_and_dispatches_record_nothing
    dispatch_beyond_the_group_limit_is_refused
    timeline_is_signalled_waited_and_read_by_the_host
    timeline_wait_times_out
    timeline_value_that_does_not_grow_is_refused
    host_signal_above_a_pending_signal_is_refused
    host_signal_below_a_promise_that_does_not_wait_for_it_is_refused
    host_signal_below_a_pending_signal_releases_its_waiter
    host_signal_below_a_promise_whose_wait_is_met_is_refused
    submit_signalling_no_more_than_it_waits_for_is_refused
    timeline_value_beyond_the_difference_limit_is_refused
    timeline_signalled_from_two_queues_without_a_wait_is_refused
    destroy_under_a_pending_submit_is_refused
    concurrent_resource_destroyed_while_another_queue_uses_it_is_refused
    destroy_after_an_unobserved_submit_is_refused
    destroy_at_once_after_the_wait_is_accepted
    destroy_racing_a_submit_refuses_one
    submits_racing_on_one_queue_keep_their_order
    host_signals_racing_on_one_timeline_keep_it_growing
    use_after_destruction_is_refused
    destroyed_after_recording_is_refused_at_submit
    handle_of_another_device_is_refused
    input_read_without_a_local_read_is_refused
    input_read_of_another_attachment_is_refused
    attachments_of_unequal_extent_are_refused
    scissor_outside_the_attachments_is_refused
    viewport_outside_its_rules_is_refused
    index_buffer_offset_outside_or_unaligned_is_refused
    indexed_draw_past_its_index_buffer_is_refused
    indirect_arguments_outside_or_unaligned_are_refused
    second_acquire_before_a_present_is_refused
    present_of_an_image_not_acquired_is_refused
    present_behind_an_unmet_wait_is_refused
    present_of_an_image_not_in_present_is_refused
    swapchain_image_on_a_frame_encoder_is_refused
    swapchain_image_used_outside_its_acquire_is_refused
    second_swapchain_is_refused
    swapchain_extent_with_a_zero_dimension_is_refused
    resize_with_a_present_pending_is_refused
    image_of_before_a_resize_is_refused
    frame_submits_complete_while_a_present_is_pending
    loss_is_returned_and_stays
    waits_return_at_once_after_a_loss
    teardown_succeeds_after_a_loss
    clean_run_sends_no_message
}

cases! {
    OUTPUT, output:
    device_memory_is_filled_through_a_staging_copy
    memoryless_g_buffer_is_lit_in_its_pass
    aliased_transients_render_in_turn
    image_round_trips_through_copies
    ownership_transfer_keeps_the_contents
    heap_is_sampled_past_slot_zero
    address_is_read_in_a_shader
    declared_reader_follows_a_lagging_writer
    sampler_filters_as_described
    specialised_constant_changes_the_output
    write_mask_keeps_a_channel
    depth_state_orders_two_triangles
    warm_cache_hits_every_stage
    two_threads_record_at_once
    triangle_matches_the_reference
    local_read_sees_what_the_pass_wrote
    clear_and_keep_load_as_described
    viewport_and_scissor_place_the_triangles
    draw_parameters_reach_the_shader
    indexed_indirect_draw_matches_the_reference
    indirect_count_draws_where_reported
    dispatch_writes_through_an_address
    dispatches_of_one_pass_see_each_others_writes
    dispatch_indirect_reads_its_counts
    each_validator_reports_its_canary
}
