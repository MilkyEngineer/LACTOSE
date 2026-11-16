//! Every refusal of the cooker, each through `cook` on a source tree of its own: the error
//! names the source file, then the node, and nothing is written.

#![forbid(unsafe_code)]

use crate::common;

use common::{Env, knight};
use lactose_cook::cook;
use lactose_core::{Error, ErrorKind};

/// The JSON of a glTF document of `nodes`, and `rest` after them (more top-level fields).
fn gltf(nodes: &str, rest: &str) -> Vec<u8> {
    format!(r#"{{"asset": {{"version": "2.0"}}, "nodes": [{nodes}]{rest}}}"#).into_bytes()
}

/// A node's mark as a bone with no role and no body.
const BONE: &str = r#""extras": {"lactose": {"bone": {}}}"#;

/// Cooks `files` under a directory of `env`'s and returns the refusal, checking that no
/// pack was written.
fn refused(env: &Env, files: &[(&str, &[u8])]) -> Error {
    let sources = env.dir.join("sources");
    env.write_tree(&sources, files);
    let pack = env.dir.join("out").join("refused.pack");
    let error = env.wait(cook(&env.io, &sources, &pack)).expect_err("the cook is refused");
    let read = env.wait(env.io.read_file(&pack)).expect_err("no pack is written");
    assert_eq!(read.kind(), ErrorKind::NotFound);
    error
}

/// `refused` for one source, `rig.gltf`, holding `nodes` and `rest`: an `InvalidData`
/// error whose message is `rig.gltf: ` then `says`.
fn assert_refused(name: &str, nodes: &str, rest: &str, says: &str) {
    let env = Env::new(name);
    let error = refused(&env, &[("rig.gltf", &gltf(nodes, rest))]);
    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    let message = error.message();
    assert!(message.starts_with("rig.gltf: "), "{message}");
    assert!(message["rig.gltf: ".len()..].starts_with(says), "{message}\n  should say {says}");
}

#[test]
fn a_glb_file_is_refused_by_its_name() {
    let env = Env::new("glb-name");
    let error =
        refused(&env, &[("knight.gltf", &knight()), ("props/crate.glb", b"glTF\x02\0\0\0")]);
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().starts_with("props/crate.glb: a binary glTF"), "{error}");
}

/// An ending in another case is neither cooked nor skipped: a case-insensitive file system
/// would make its asset's name differ from the one a case-sensitive one gives.
#[test]
fn an_ending_in_another_case_is_refused_with_its_rename() {
    for (name, file, renamed) in
        [("case-gltf", "Squire.GLTF", "Squire.gltf"), ("case-glb", "props/crate.Glb", "crate.glb")]
    {
        let env = Env::new(name);
        let error = refused(&env, &[("knight.gltf", &knight()), (file, &knight())]);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        let says = format!("{file}: an ending the cooker reads in lowercase alone");
        assert!(error.message().starts_with(&says), "{error}");
        assert!(error.message().ends_with(&format!("rename it `{renamed}`")), "{error}");
    }
    // A name that is only an ending, in any case, would cook to an asset whose file name is
    // empty: it is refused, not renamed to another name that is only an ending.
    for (name, file) in
        [("only-gltf", ".gltf"), ("only-gltf-case", ".GLTF"), ("only-glb", "props/.glb")]
    {
        let env = Env::new(name);
        let error = refused(&env, &[("knight.gltf", &knight()), (file, &knight())]);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        let says = format!("{file}: a name that is only an ending");
        assert!(error.message().starts_with(&says), "{error}");
    }
}

#[test]
fn a_glb_named_gltf_is_refused_by_its_magic() {
    let env = Env::new("glb-magic");
    let error = refused(&env, &[("knight.gltf", b"glTF\x02\0\0\0\x0c\0\0\0")]);
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().starts_with("knight.gltf: a binary glTF"), "{error}");
}

/// Rest poses are the nodes' transforms, so no buffer is read: the knight's `knight.bin`,
/// which holds its inverse bind matrices and mesh, is not committed, and it still cooks.
#[test]
fn a_source_whose_buffers_are_missing_still_cooks() {
    let env = Env::new("no-buffers");
    let sources = env.dir.join("sources");
    env.write_tree(&sources, &[("knight.gltf", &knight())]);
    let pack = env.dir.join("knight.pack");
    let published = env.wait(cook(&env.io, &sources, &pack)).unwrap();
    assert_eq!(published.len(), 1);
}

#[test]
fn a_non_uniform_scale_on_a_bone_is_refused() {
    assert_refused(
        "non-uniform",
        &format!(r#"{{"name": "hips", "scale": [1, 2, 1], {BONE}}}"#),
        "",
        "node 0 `hips`: the non-uniform scale [1.0, 2.0, 1.0]",
    );
}

#[test]
fn a_uniform_scale_other_than_one_above_a_bone_is_refused() {
    // On the unmarked node composed into the root bone's rest: an exporter's
    // centimetre armature.
    assert_refused(
        "non-unit",
        &format!(
            r#"{{"name": "Armature", "scale": [0.01, 0.01, 0.01], "children": [1]}},
               {{"name": "hips", {BONE}}}"#
        ),
        "",
        "node 0 `Armature`: the scale 0.01 on every axis, not one: a sim bone's space is not \
         scaled; apply the scale to the armature before export",
    );
    // A negative factor: its square is one, but the scale turns or mirrors.
    assert_refused(
        "negative",
        &format!(
            r#"{{"name": "Armature", "scale": [1, -1, -1], "children": [1]}},
               {{"name": "hips", {BONE}}}"#
        ),
        "",
        "node 0 `Armature`: the non-uniform scale [1.0, -1.0, -1.0]",
    );
}

#[test]
fn a_matrix_that_scales_or_mirrors_is_refused() {
    assert_refused(
        "matrix-scale",
        &format!(r#"{{"name": "hips", "matrix": [2,0,0,0, 0,2,0,0, 0,0,2,0, 0,0,0,1], {BONE}}}"#),
        "",
        "node 0 `hips`: a matrix that scales",
    );
    assert_refused(
        "matrix-mirror",
        &format!(r#"{{"name": "hips", "matrix": [-1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1], {BONE}}}"#),
        "",
        "node 0 `hips`: a matrix that mirrors",
    );
}

#[test]
fn a_cycle_is_refused() {
    assert_refused(
        "cycle",
        &format!(r#"{{"name": "a", "children": [1], {BONE}}}, {{"name": "b", "children": [0]}}"#),
        "",
        "node 0 `a`: its parents form a cycle",
    );
    assert_refused(
        "self-cycle",
        &format!(r#"{{"name": "a", "children": [0], {BONE}}}"#),
        "",
        "node 0 `a`: its parents form a cycle",
    );
}

#[test]
fn a_second_root_is_refused() {
    assert_refused(
        "second-root",
        &format!(
            r#"{{"name": "a", {BONE}}}, {{"name": "between", "children": [2]}},
               {{"name": "b", {BONE}}}"#
        ),
        "",
        "node 2 `b`: a second root bone, beside node 0 `a`",
    );
}

/// A chain of `count` bones, `b0` at the top.
fn chain(count: usize) -> String {
    (0..count)
        .map(|i| {
            let children =
                if i + 1 < count { format!(r#""children": [{}], "#, i + 1) } else { String::new() };
            format!(r#"{{"name": "b{i}", {children}{BONE}}}"#)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn a_seventeenth_bone_is_refused_and_sixteen_cook() {
    assert_refused(
        "seventeen",
        &chain(17),
        "",
        "node 16 `b16`: bone 17: a sim skeleton has at most 16",
    );
    let env = Env::new("sixteen");
    let sources = env.dir.join("sources");
    env.write_tree(&sources, &[("rig.gltf", &gltf(&chain(16), ""))]);
    env.wait(cook(&env.io, &sources, &env.dir.join("rig.pack"))).expect("sixteen bones cook");
}

/// A struct written as an array, its fields by position, is refused wherever the reader
/// expects an object: in the mark at every level, and in the document.
#[test]
fn an_object_written_as_an_array_is_refused_at_every_level() {
    const ARRAY: &str = "invalid type: sequence, expected a JSON object";
    let collider = r#""shape": {"sphere": {"radius": 0.5}}, "membership": 1, "filter": 2"#;
    let marks = [
        r#"{"bone": ["hips", null]}"#.to_owned(),
        r#"{"socket": []}"#.to_owned(),
        format!(
            r#"{{"bone": {{"body": [null, 1, null, null, null, null, null, {{{collider}}}]}}}}"#
        ),
        r#"{"bone": {"body": {"collider": [{"sphere": {"radius": 0.5}}, 1, 2]}}}"#.to_owned(),
        r#"{"bone": {"body": {"collider": {"shape": {"sphere": [0.5]}, "membership": 1,
                                          "filter": 2}}}}"#
            .to_owned(),
        format!(r#"{{"bone": {{"body": {{"collider": {{{collider}}}, "joint": ["fixed"]}}}}}}"#),
    ];
    for (index, mark) in marks.iter().enumerate() {
        assert_refused(
            &format!("array-mark-{index}"),
            &format!(r#"{{"name": "hips", "extras": {{"lactose": {mark}}}}}"#),
            "",
            &format!("node 0 `hips`: `extras.lactose`: {ARRAY}"),
        );
    }
    // The socket is node 1, under a bone: its array is the refusal, not a missing bone.
    assert_refused(
        "array-socket",
        &format!(
            r#"{{"name": "hips", "children": [1], {BONE}}},
               {{"name": "grip", "extras": {{"lactose": {{"socket": []}}}}}}"#
        ),
        "",
        &format!("node 1 `grip`: `extras.lactose`: {ARRAY}"),
    );
    for (name, source) in [
        ("array-node", gltf(r#"["hips"]"#, "")),
        ("array-asset", br#"{"asset": ["2.0"], "nodes": []}"#.to_vec()),
        ("array-skin", gltf(&format!(r#"{{"name": "hips", {BONE}}}"#), r#", "skins": [[[0]]]"#)),
        ("array-document", br#"[{"version": "2.0"}]"#.to_vec()),
    ] {
        let env = Env::new(name);
        let error = refused(&env, &[("rig.gltf", &source)]);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{name}: {error}");
        let message = error.message();
        assert!(message.starts_with("rig.gltf: not the JSON of a glTF document: "), "{message}");
        assert!(message.contains(ARRAY), "{name}: {message}");
    }
}

/// A key written twice in one object of the mark is refused, the path named: serde_json
/// would keep the last silently. A key repeated elsewhere in `extras` is not the cooker's.
#[test]
fn a_key_written_twice_in_the_mark_is_refused() {
    for (name, extras, path) in [
        ("twice-mark", r#"{"lactose": {"bone": {}}, "lactose": {"socket": {}}}"#, "lactose"),
        (
            "twice-role",
            r#"{"lactose": {"bone": {"role": "hips", "role": "head"}}}"#,
            "lactose.bone.role",
        ),
        (
            "twice-radius",
            r#"{"lactose": {"bone": {"body": {"collider": {
                "shape": {"sphere": {"radius": 1, "radius": 2}},
                "membership": 1, "filter": 2}}}}}"#,
            "lactose.bone.body.collider.shape.sphere.radius",
        ),
        ("twice-variant", r#"{"lactose": {"bone": {}, "bone": {}}}"#, "lactose.bone"),
    ] {
        assert_refused(
            name,
            &format!(r#"{{"name": "hips", "extras": {extras}}}"#),
            "",
            &format!("node 0 `hips`: `extras.{path}` is written twice"),
        );
    }
    let env = Env::new("twice-elsewhere");
    let sources = env.dir.join("sources");
    let nodes = r#"{"name": "hips", "extras": {"tool": 1, "tool": 2, "lactose": {"bone": {}}}}"#;
    env.write_tree(&sources, &[("rig.gltf", &gltf(nodes, ""))]);
    let published = env.wait(cook(&env.io, &sources, &env.dir.join("rig.pack"))).unwrap();
    assert_eq!(published.len(), 1);
}

/// The refusals the brief does not name: the rest of the reader's and the schema's rules,
/// and one of `Skeleton::validate`'s, named by node.
#[test]
fn every_other_source_refusal_names_the_file_and_the_node() {
    let cases: [(&str, String, &str, &str); 11] = [
        (
            "two-parents",
            r#"{"name": "a", "children": [2]}, {"name": "b", "children": [2]}, {"name": "c"}"#
                .into(),
            "",
            "node 2 `c`: a child of both node 0 `a` and node 1 `b`",
        ),
        (
            "child-past",
            r#"{"name": "a", "children": [5]}"#.into(),
            "",
            "node 0 `a`: child 5 is past",
        ),
        (
            "not-a-joint",
            format!(r#"{{"name": "a", "children": [1], {BONE}}}, {{"name": "b", {BONE}}}"#),
            r#", "skins": [{"joints": [0]}]"#,
            "node 1 `b`: a bone that is no joint of the skins",
        ),
        ("unnamed", format!("{{{BONE}}}"), "", "node 0: marked, with no name"),
        (
            "socket-alone",
            r#"{"name": "a", "extras": {"lactose": {"socket": {}}}}"#.into(),
            "",
            "node 0 `a`: a socket under no bone",
        ),
        ("no-bone", r#"{"name": "a"}"#.into(), "", "no node is marked as a bone"),
        (
            "rotation",
            format!(r#"{{"name": "a", "rotation": [0, 0, 0, 2], {BONE}}}"#),
            "",
            "node 0 `a`: the rotation [0.0, 0.0, 0.0, 2.0] is not a unit quaternion",
        ),
        (
            "matrix-and-trs",
            format!(
                r#"{{"name": "a", "translation": [0, 1, 0],
                     "matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1], {BONE}}}"#
            ),
            "",
            "node 0 `a`: both a matrix and a translation",
        ),
        (
            "unknown-field",
            r#"{"name": "a", "extras": {"lactose": {"bone": {"roll": "hips"}}}}"#.into(),
            "",
            "node 0 `a`: `extras.lactose`: unknown field `roll`",
        ),
        (
            "groups-meet",
            r#"{"name": "a", "extras": {"lactose": {"bone": {"body": {"collider": {
                "shape": {"sphere": {"radius": 0.25}}, "membership": 1, "filter": 1}}}}}}"#
                .into(),
            "",
            "node 0 `a`: the bones' memberships 0x1 meet their filters 0x1",
        ),
        (
            "same-key",
            format!(r#"{{"name": "a", "children": [1], {BONE}}}, {{"name": "a", {BONE}}}"#),
            "",
            "node 1 `a`: the bone key of node 0 `a`",
        ),
    ];
    for (name, nodes, rest, says) in &cases {
        assert_refused(name, nodes, rest, says);
    }
    let env = Env::new("version");
    let error = refused(&env, &[("rig.gltf", br#"{"asset": {"version": "1.0"}}"#)]);
    assert_eq!(error.message(), "rig.gltf: glTF version 1.0, where the reader reads version 2");
    let env = Env::new("not-json");
    let error = refused(&env, &[("rig.gltf", b"{\"asset\":")]);
    assert!(error.message().starts_with("rig.gltf: not the JSON of a glTF document"), "{error}");
}

#[test]
fn a_pack_name_without_the_ending_an_empty_tree_and_a_link_are_refused() {
    let env = Env::new("pack-name");
    let sources = env.dir.join("sources");
    env.write_tree(&sources, &[("knight.gltf", &knight())]);
    let error = env.wait(cook(&env.io, &sources, &env.dir.join("knight.pak"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert!(error.message().starts_with("the pack's file name ends in `.pack`"), "{error}");

    let env = Env::new("empty");
    let error = refused(&env, &[("readme.txt", b"no sources")]);
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");

    #[cfg(unix)]
    {
        let env = Env::new("link");
        let sources = env.dir.join("sources");
        env.write_tree(&sources, &[("knight.gltf", &knight())]);
        std::os::unix::fs::symlink(sources.join("knight.gltf"), sources.join("linked.gltf"))
            .unwrap();
        let error = env.wait(cook(&env.io, &sources, &env.dir.join("out.pack"))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(error.message().starts_with("linked.gltf: neither a file nor"), "{error}");
    }
}

#[test]
fn the_binary_exits_one_with_the_refusal_and_two_with_the_usage() {
    let env = Env::new("binary");
    let sources = env.dir.join("sources");
    env.write_tree(&sources, &[("rig.gltf", &gltf(r#"{"name": "a"}"#, ""))]);
    let pack = env.dir.join("rig.pack");
    let binary = || std::process::Command::new(env!("CARGO_BIN_EXE_ricotta"));
    let output = binary().arg(&sources).arg(&pack).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("ricotta: InvalidData: rig.gltf: no node is marked as a bone"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
    let output = binary().arg(&sources).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "usage: ricotta SOURCES PACK\n");
}
