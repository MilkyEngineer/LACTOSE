//! What runs against a facade: the plain data whose bodies are real, the numbers the
//! shaders and the host agree on, and the README's naming of every public item.

use lactose_ecs::WorldKind;
use lactose_present_api::RenderTransform;
use lactose_renderer::{
    CameraData, FrameStats, InstanceRecord, LOG_TARGET, Opened, ScatterEntry, Slot, StagingRow,
    shader,
};
use lactose_rhi_api::{IndexedIndirectArgs, PUSH_BYTES_MAX};

#[test]
fn a_slot_is_its_index() {
    assert_eq!(Slot::new(7).index(), 7);
    assert!(Slot::new(1) < Slot::new(2));
    assert_eq!(Slot::new(3), Slot::new(3));
}

#[test]
fn counts_start_at_zero_with_no_kind_lost() {
    let stats = FrameStats::default();
    assert_eq!((stats.pass_rows, stats.remote_rows, stats.dropped), (0, 0, 0));
    assert_eq!((stats.duplicates, stats.unplaced, stats.entries), (0, 0, 0));
    assert_eq!((stats.allocated, stats.freed, stats.live), (0, 0, 0));
    assert!(!stats.any_lost());
    for kind in [WorldKind::Server, WorldKind::Client, WorldKind::Prediction] {
        assert!(!stats.lost(kind));
        assert_eq!((stats.dropped_in(kind), stats.unplaced_in(kind)), (0, 0));
    }
}

#[test]
fn an_open_result_compares_by_its_point() {
    assert_eq!(Opened::Ready, Opened::Ready);
    assert_ne!(Opened::Ready, Opened::Unsubmitted);
}

#[test]
fn the_records_are_the_sizes_the_readme_gives() {
    // "Formats": a placement is forty bytes, and the records are built from it.
    assert_eq!(size_of::<RenderTransform>(), 40);
    assert_eq!(StagingRow::BYTES, 48);
    assert_eq!(ScatterEntry::BYTES, 12);
    assert_eq!(InstanceRecord::BYTES, 128);
    assert_eq!(CameraData::BYTES, 80);
    assert_eq!(IndexedIndirectArgs::BYTES, 20);
    // The instance buffer's stride keeps a record's 64-bit address arithmetic aligned.
    assert_eq!(InstanceRecord::BYTES % 8, 0);
    assert_eq!(StagingRow::BYTES % 8, 0);
}

#[test]
fn the_flags_are_distinct_bits() {
    let entry = [ScatterEntry::FREE, ScatterEntry::TAKE_BASE, ScatterEntry::HAS_BASE];
    for (index, flag) in entry.iter().enumerate() {
        assert_eq!(flag.count_ones(), 1);
        assert!(entry[..index].iter().all(|other| other & flag == 0));
    }
    assert_eq!(StagingRow::NO_INTERPOLATE.count_ones(), 1);
    assert_eq!(InstanceRecord::LIVE.count_ones(), 1);
}

#[test]
fn the_push_blocks_are_three_addresses_and_two_words() {
    // Three addresses of eight bytes, a word and a word of zero. That both fit every
    // backend's block is a static assertion of `src/lib.rs`.
    assert_eq!(shader::SCATTER_PUSH_BYTES, 3 * 8 + 4 + 4);
    assert_eq!(shader::TRIANGLE_PUSH_BYTES, 3 * 8 + 4 + 4);
    assert_eq!(PUSH_BYTES_MAX, 128);
    assert_eq!(shader::SCATTER_GROUP, 64);
    assert_eq!(shader::CELL_SIZE_CONSTANT, 0);
}

#[test]
fn the_triangle_faces_plus_z() {
    let [a, b, c] = shader::TRIANGLE;
    // The z of the cross product of two edges: positive for an anticlockwise triangle
    // seen from `+Z`.
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    assert!(area > 0.0);
    assert!(shader::TRIANGLE.iter().all(|vertex| vertex[2] == 0.0));
}

#[test]
fn no_palette_colour_is_the_clear_colour_and_no_two_are_equal() {
    let clear = shader::CLEAR.map(|channel| (channel * 255.0) as u8);
    assert_eq!(clear, [0, 0, 0, 255]);
    for (index, colour) in shader::PALETTE.iter().enumerate() {
        assert_ne!(*colour, clear);
        assert_eq!(colour[3], 255);
        assert!(shader::PALETTE[..index].iter().all(|other| other != colour));
    }
}

#[test]
fn the_log_target_is_the_codename() {
    assert_eq!(LOG_TARGET, "cheddar");
}

/// The words of `text` that begin with an upper-case letter: the names of types, traits
/// and constants in a `pub use` or a `pub const` line.
fn names(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
}

/// Every public item of the crate is named in the README's surface table, so the contract
/// and the surface cannot drift apart unseen. The items are read from `src/lib.rs`'s
/// re-exports, the `graph` module's among them, and from the constants of `src/shader.rs`.
#[test]
fn the_readme_names_every_public_item() {
    const README: &str = include_str!("../README.md");
    const LIB: &str = include_str!("../src/lib.rs");
    const SHADER: &str = include_str!("../src/shader.rs");

    let surface = README
        .split("## Surface")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the README has a Surface section");

    // The re-exports: every line from the `graph` module to the static assertions that
    // is not a comment.
    let exports: Vec<&str> = LIB
        .lines()
        .skip_while(|line| !line.starts_with("pub mod graph"))
        .take_while(|line| !line.starts_with("// What the contract promises"))
        .filter(|line| !line.trim_start().starts_with("//"))
        .flat_map(names)
        .collect();
    assert_eq!(exports.len(), 17 + 26, "a re-export was added or removed: count it here");

    let constants: Vec<&str> = SHADER
        .lines()
        .filter_map(|line| line.strip_prefix("pub const "))
        .filter_map(|rest| rest.split(':').next())
        .collect();
    assert_eq!(constants.len(), 7, "a shader constant was added or removed: count it here");

    for item in exports.iter().chain(&constants) {
        assert!(surface.contains(&format!("`{item}`")), "the surface table does not name `{item}`");
    }
    // The two modules are named too.
    assert!(surface.contains("module `graph`") && surface.contains("module `shader`"));
}
