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
    // The refusals every backend makes: the README's list, by count.
    let everywhere = CASES.iter().filter(|case| case.devices && case.refuses.is_some());
    assert_eq!(everywhere.count(), 19);
    // Every rule has a contract case that fails on a backend built without it.
    for rule in Rule::ALL {
        assert!(
            CASES.iter().any(|case| case.refuses == Some(rule)),
            "no case provokes `{}`",
            rule.name()
        );
    }
}
