//! The cooked pack set of a fresh process (`README.md`, "Mounting"): the `.pack` files
//! directly in a directory, mounted in byte order of their names, so the store names the
//! versions of the last name, whatever order the packs were written in and whatever order
//! the mounts end in.

#![forbid(unsafe_code)]

use crate::common;

use common::*;
use lactose_asset::{Storage, pack_set};
use lactose_asset_api::{AssetCatalog, LoadPriority, Readiness};
use lactose_core::ErrorKind;

#[test]
fn a_fresh_process_mounts_the_set_in_name_order_and_names_the_last_names_versions() {
    let env = Env::new("pack_set");
    let cooked = env.dir.join("cooked");
    // A previous run's files would be read as this run's.
    match std::fs::remove_dir_all(&cooked) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("{} not removed: {error}", cooked.display()),
    }
    env.wait(env.io.create_dir_all(cooked.clone())).unwrap();
    let x = id("x");
    let [base, patch, next] = ["base", "patch", "next"].map(|content| text(content.as_bytes(), []));

    // The later name is written first: the order is the names', not the writes'.
    env.write_at(cooked.join("1-patch.pack"), pack(&[(x, patch.clone())], Storage::Raw));
    env.write_at(cooked.join("0-base.pack"), pack(&[(x, base.clone())], Storage::Raw));
    // Not in the set, each of which would change the answer or fail the mounts if it were:
    // a directory named as a pack, another file, and a replace's temporary holding a pack.
    env.wait(env.io.create_dir_all(cooked.join("8-old.pack"))).unwrap();
    env.write_at(cooked.join("notes.txt"), b"not a pack".to_vec());
    env.write_at(
        cooked.join("2-next.pack.0123456789abcdef.tmp"),
        pack(&[(x, next.clone())], Storage::Raw),
    );
    // And a symbolic link to the base, named to sort last.
    #[cfg(unix)]
    std::os::unix::fs::symlink(cooked.join("0-base.pack"), cooked.join("9-link.pack")).unwrap();

    let set = env.wait(pack_set(&env.io, &cooked)).unwrap();
    assert_eq!(set, [cooked.join("0-base.pack"), cooked.join("1-patch.pack")]);

    // A fresh store mounts the set in that order; the mounts end in the other.
    let (loaders, _opener) = test_loaders();
    let store = env.store(loaders);
    let mounts: Vec<_> = set.iter().map(|pack| store.mount(pack)).collect();
    for mount in mounts.into_iter().rev() {
        env.wait(mount).expect("the pack mounts");
    }
    store.request_newest(x, LoadPriority(0));
    env.settle(&store);
    let newest = store.newest(x).map(|handle| handle.version());
    assert_eq!(newest, Readiness::Ready(patch.version()));
    assert_ne!(patch.version(), base.version());

    let error = env.wait(pack_set(&env.io, env.dir.join("no-such-directory"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}
