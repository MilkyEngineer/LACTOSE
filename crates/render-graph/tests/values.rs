//! What runs against a facade: the history ping-pong, which is plain data, and the
//! README's naming of every public item.

use lactose_render_graph::{History, LOG_TARGET};

#[test]
fn a_new_history_writes_its_first_half_and_has_no_previous() {
    let history = History::new(['a', 'b']);
    assert_eq!(history.current(), 'a');
    assert_eq!(history.previous(), None);
}

#[test]
fn advance_swaps_the_halves() {
    let mut history = History::new(['a', 'b']);
    history.advance();
    assert_eq!(history.current(), 'b');
    assert_eq!(history.previous(), Some('a'));
    history.advance();
    assert_eq!(history.current(), 'a');
    assert_eq!(history.previous(), Some('b'));
}

#[test]
fn reset_forgets_the_previous_half_and_keeps_the_current_one() {
    let mut history = History::new([1u32, 2]);
    history.advance();
    history.reset();
    assert_eq!(history.current(), 2);
    assert_eq!(history.previous(), None);
    // The frame after a reset writes the half the reset left current, and the one after
    // reads it.
    history.advance();
    assert_eq!(history.current(), 1);
    assert_eq!(history.previous(), Some(2));
}

#[test]
fn a_history_never_hands_out_one_half_as_both() {
    let mut history = History::new([10u8, 20]);
    for _ in 0..5 {
        history.advance();
        assert_ne!(Some(history.current()), history.previous());
    }
}

#[test]
fn a_history_is_usable_in_a_constant() {
    const HISTORY: History<u8> = {
        let mut history = History::new([1, 2]);
        history.advance();
        history
    };
    assert_eq!(HISTORY.current(), 2);
    assert_eq!(HISTORY.previous(), Some(1));
}

#[test]
fn the_log_target_is_the_codename() {
    assert_eq!(LOG_TARGET, "gouda");
}

/// Every public item of the crate is named in the README's surface table, so the contract
/// and the surface cannot drift apart unseen. The list is the `pub use` lines of
/// `src/lib.rs`; an item added there is added here and to the table.
#[test]
fn the_readme_names_every_public_item() {
    const README: &str = include_str!("../README.md");
    const ITEMS: [&str; 28] = [
        "FrameDecl",
        "GraphDesc",
        "GraphStats",
        "LOG_TARGET",
        "RecordedFrame",
        "RenderGraph",
        "History",
        "BufferId",
        "ImageId",
        "PassId",
        "BufferAccess",
        "BufferUse",
        "ColourUse",
        "ComputePassDecl",
        "CopyPassDecl",
        "DepthUse",
        "RenderPassDecl",
        "CompiledFrame",
        "CompiledPlan",
        "GraphKey",
        "PlannedPass",
        "ComputeRecorder",
        "CopyRecorder",
        "RenderRecorder",
        "BufferImport",
        "ImageImport",
        "TransientImage",
        "TransientMemory",
    ];
    let surface = README
        .split("## Surface")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the README has a Surface section");
    for item in ITEMS {
        assert!(surface.contains(&format!("`{item}`")), "the surface table does not name `{item}`");
    }
    // And the list above is the whole of `src/lib.rs`'s re-exports.
    const LIB: &str = include_str!("../src/lib.rs");
    let exported: usize = LIB
        .lines()
        .skip_while(|line| !line.starts_with("pub use "))
        .take_while(|line| !line.starts_with("// What the contract promises"))
        .map(|line| {
            line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
                .count()
        })
        .sum();
    assert_eq!(exported, ITEMS.len(), "a re-export this test does not list");
}
