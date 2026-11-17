//! The bodies the facade does have: the rule names, the refusal's round trip, the queue
//! layout, the indirect record and the conformance table's own consistency.

use std::collections::BTreeSet;

use lactose_core::{Error, ErrorKind};
use lactose_rhi_api::conformance::{CASES, CaseKind};
use lactose_rhi_api::{IndexedIndirectArgs, QueueLayout, QueuePath, QueueRole, Rule};

#[test]
fn rule_names_are_distinct_and_kept() {
    let names: BTreeSet<&str> = Rule::ALL.iter().map(|rule| rule.name()).collect();
    assert_eq!(names.len(), Rule::ALL.len());
    // The names the two result files give the rules they found.
    for name in
        ["local", "split", "no-qfot", "heap-layout", "uuwp-used", "memoryless-store", "no-fence"]
    {
        assert!(names.contains(name), "{name}");
    }
}

#[test]
fn refusal_names_its_rule_and_nothing_else_does() {
    for rule in Rule::ALL {
        let error = rule.refuse("a detail");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(Rule::of(&error), Some(rule));
        assert!(error.message().starts_with(rule.name()), "{}", error.message());
    }
    assert_eq!(Rule::of(&Error::new(ErrorKind::DeviceLost, "lost")), None);
    assert_eq!(Rule::of(&Error::new(ErrorKind::InvalidData, "bytes")), None);
}

#[test]
fn a_wrapped_refusal_keeps_its_rule() {
    let refusal = Rule::Pending.refuse("a destroy under a pending submit");
    let once = Error::new(ErrorKind::InvalidData, "the frame's teardown").with_source(refusal);
    assert_eq!(Rule::of(&once), Some(Rule::Pending));
    let twice = Error::new(ErrorKind::InvalidData, "the render side").with_source(once);
    assert_eq!(Rule::of(&twice), Some(Rule::Pending));
}

#[test]
fn roles_resolve_by_the_layout() {
    let roles = [QueueRole::Graphics, QueueRole::Compute, QueueRole::Present];
    for role in roles {
        assert_eq!(QueueLayout::ONE.serving(role), QueueRole::Graphics);
        assert_eq!(QueueLayout::SEVERAL.serving(role), role);
        assert!(QueueLayout::ONE.same_queue(role, QueueRole::Graphics));
    }
    let present_alone = QueueLayout { compute: QueuePath::Shared, present: QueuePath::Own };
    assert!(present_alone.same_queue(QueueRole::Graphics, QueueRole::Compute));
    assert!(!present_alone.same_queue(QueueRole::Graphics, QueueRole::Present));
    assert!(!QueueLayout::SEVERAL.same_queue(QueueRole::Compute, QueueRole::Present));
}

#[test]
fn indirect_record_is_five_little_endian_words() {
    let args = IndexedIndirectArgs {
        index_count: 3,
        instance_count: 0x0102_0304,
        first_index: 7,
        base_vertex: -1,
        first_instance: 9,
    };
    let bytes = args.to_bytes();
    assert_eq!(bytes[0..4], [3, 0, 0, 0]);
    assert_eq!(bytes[4..8], [4, 3, 2, 1]);
    assert_eq!(bytes[8..12], [7, 0, 0, 0]);
    assert_eq!(bytes[12..16], [0xff; 4]);
    assert_eq!(bytes[16..20], [9, 0, 0, 0]);
}

/// The refusal cases a device backend runs: one or more for each clause of the rules
/// table's "Every backend" column.
const EVERYWHERE: [&str; 55] = [
    "unreported_capability_is_refused",
    "submit_to_the_present_role_is_refused",
    "overlapping_host_access_is_refused",
    "destroy_during_host_access_is_refused",
    "host_range_outside_the_buffer_is_refused",
    "host_access_outside_the_memory_class_is_refused",
    "address_of_a_buffer_without_the_usage_is_refused",
    "read_under_a_pending_writer_is_refused",
    "write_under_a_pending_writer_is_refused",
    "submit_writing_under_a_host_access_is_refused",
    "buffer_use_outside_its_usage_is_refused",
    "image_use_outside_its_usage_is_refused",
    "memoryless_request_without_transient_is_refused",
    "memoryless_attachment_stored_is_refused",
    "memoryless_attachment_kept_is_refused",
    "placement_outside_the_block_is_refused",
    "destroy_of_a_block_an_image_is_placed_in_is_refused",
    "heap_slot_outside_the_heap_is_refused",
    "declared_slot_empty_is_refused",
    "heap_write_under_a_pending_reader_is_refused",
    "destroy_of_an_image_a_slot_holds_is_refused",
    "sampler_destroyed_under_a_pending_submit_is_refused",
    "undeclared_slot_argument_is_refused",
    "undeclared_address_argument_is_refused",
    "shader_needing_an_unenabled_capability_is_refused",
    "pipeline_of_other_formats_than_its_pass_is_refused",
    "encoder_reset_while_pending_is_refused",
    "encoder_in_two_submits_is_refused",
    "encoder_resubmitted_without_reset_is_refused",
    "encoder_of_another_role_or_device_is_refused",
    "command_outside_its_scope_is_refused",
    "draw_with_nothing_bound_is_refused",
    "pass_left_open_is_refused",
    "push_outside_the_block_is_refused",
    "copy_region_outside_its_resource_is_refused",
    "dispatch_beyond_the_group_limit_is_refused",
    "timeline_value_that_does_not_grow_is_refused",
    "host_signal_above_a_pending_signal_is_refused",
    "host_signal_below_a_promise_that_does_not_wait_for_it_is_refused",
    "timeline_signalled_from_two_queues_without_a_wait_is_refused",
    "destroy_under_a_pending_submit_is_refused",
    "concurrent_resource_destroyed_while_another_queue_uses_it_is_refused",
    "use_after_destruction_is_refused",
    "destroyed_after_recording_is_refused_at_submit",
    "attachments_of_unequal_extent_are_refused",
    "scissor_outside_the_attachments_is_refused",
    "viewport_outside_its_rules_is_refused",
    "index_buffer_offset_outside_or_unaligned_is_refused",
    "indexed_draw_past_its_index_buffer_is_refused",
    "indirect_arguments_outside_or_unaligned_are_refused",
    "second_acquire_before_a_present_is_refused",
    "present_of_an_image_not_acquired_is_refused",
    "second_swapchain_is_refused",
    "resize_with_a_present_pending_is_refused",
    "image_of_before_a_resize_is_refused",
];

#[test]
fn readme_names_every_case_and_every_device_refusal() {
    let readme = include_str!("../README.md");
    for case in CASES {
        assert!(readme.contains(&format!("`{}`", case.name)), "{}", case.name);
    }
    // The paragraph of the README that lists the refusals a device runs.
    let start = readme.find("The refusal cases a device backend also runs").expect("the list");
    let end = readme[start..].find("\n\n").expect("its end");
    let list = &readme[start..start + end];
    for name in EVERYWHERE {
        assert!(list.contains(&format!("`{name}`")), "{name}");
    }
}

#[test]
fn conformance_table_is_consistent() {
    let names: BTreeSet<&str> = CASES.iter().map(|case| case.name).collect();
    assert_eq!(names.len(), CASES.len(), "a case name is used twice");
    for case in CASES {
        assert!(
            case.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "{}",
            case.name
        );
        // Only a contract case provokes a refusal: a device backend need not check a rule.
        assert!(case.kind == CaseKind::Contract || case.refuses.is_none(), "{}", case.name);
        // The only cases a device backend does not run are refusals null alone makes.
        assert!(case.devices || case.refuses.is_some(), "{}", case.name);
    }
    // The refusals every backend makes, by name.
    let everywhere: BTreeSet<&str> = CASES
        .iter()
        .filter(|case| case.devices && case.refuses.is_some())
        .map(|case| case.name)
        .collect();
    assert_eq!(everywhere, BTreeSet::from(EVERYWHERE));
    // Every rule has a contract case that fails on a backend built without it.
    for rule in Rule::ALL {
        assert!(
            CASES.iter().any(|case| case.refuses == Some(rule)),
            "no case provokes `{}`",
            rule.name()
        );
    }
}
