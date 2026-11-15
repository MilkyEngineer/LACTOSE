//! The cooked pack set of a fresh process (`README.md`, "The cooked pack set"): the `.pack`
//! files directly in the directory, mounted in byte order of their names, so the store names
//! the versions of the last name, whatever order they were cooked in and whatever order the
//! mounts end in.

#![forbid(unsafe_code)]

mod common;

use common::{Env, knight};
use lactose_anim::SkeletonLoader;
use lactose_asset::Store;
use lactose_asset_api::{AssetCatalog, AssetId, ErasedLoader, LoadPriority};
use lactose_cook::{cook, pack_set};

#[test]
fn a_fresh_process_mounts_the_set_in_name_order_and_names_the_last_names_versions() {
    let env = Env::new("pack_set");
    let knight = knight();
    let mut patched = knight.clone();
    let at = patched.windows(7).position(|w| w == b"[0, 1, ").expect("the hips' translation");
    patched[at..at + 7].copy_from_slice(b"[0, 2, ");
    let base_sources = env.dir.join("base");
    let patch_sources = env.dir.join("patch");
    env.write_tree(&base_sources, &[("knight.gltf", &knight)]);
    env.write_tree(&patch_sources, &[("knight.gltf", &patched)]);

    // The later name is cooked first: the order is the names', not the cooks'.
    let cooked = env.dir.join("cooked");
    let patch = env.wait(cook(&env.io, &patch_sources, &cooked.join("1-patch.pack"))).unwrap();
    let base = env.wait(cook(&env.io, &base_sources, &cooked.join("0-base.pack"))).unwrap();
    let id = AssetId::from_name("knight.skeleton");
    assert_eq!((base[0].id, patch[0].id), (id, id));
    assert_ne!(base[0].version, patch[0].version);
    // Not in the set: a directory, another file, and a replace's temporary.
    env.wait(env.io.create_dir_all(cooked.join("old.pack"))).unwrap();
    env.write_tree(
        &cooked,
        &[("notes.txt", b"not a pack"), ("2-next.pack.0123456789abcdef.tmp", b"half a pack")],
    );

    let set = env.wait(pack_set(&env.io, &cooked)).unwrap();
    assert_eq!(set, [cooked.join("0-base.pack"), cooked.join("1-patch.pack")]);

    // A fresh store mounts the set in that order; the mounts end in the other.
    let store =
        Store::new(env.executor.clone(), env.io.clone(), vec![ErasedLoader::new(SkeletonLoader)])
            .unwrap();
    let mounts: Vec<_> = set.iter().map(|pack| store.mount(pack)).collect();
    for mount in mounts.into_iter().rev() {
        env.wait(mount).expect("the pack mounts");
    }
    store.request_newest(id, LoadPriority(0));
    env.wait(store.settled());
    let newest = store.newest(id).ready().expect("the knight loads");
    assert_eq!(newest.version(), patch[0].version);
}
