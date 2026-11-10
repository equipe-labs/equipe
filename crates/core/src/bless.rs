//! Tests that write their pinned files again when asked (`EQUIPE_BLESS_*`,
//! `EQUIPE_WRITE_VECTORS`) write into their crate's folder as it was when the
//! test was built. With a `CARGO_TARGET_DIR` shared between worktrees, a test
//! built in one worktree can run from another, and would overwrite the first
//! one's files. Before writing, such a test checks it runs where it was built.

use std::path::{Path, PathBuf};

/// Stop unless this test runs from the crate folder it was built in. `built`
/// is `env!("CARGO_MANIFEST_DIR")`; cargo sets the same variable when it runs
/// a test, so the two differ when the binary came from another worktree.
#[doc(hidden)]
pub fn here(built: &str) {
    if let Err(why) = check(built, std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).as_deref()) {
        panic!("{why}");
    }
}

/// Why writing into `built` would be wrong, when run from `running`.
fn check(built: &str, running: Option<&Path>) -> Result<(), String> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    match running {
        Some(running) if canon(running) == canon(Path::new(built)) => Ok(()),
        Some(running) => Err(format!(
            "not writing pinned files: this test was built in {built} but runs from {}. A shared CARGO_TARGET_DIR ran another \
worktree's binary; run it again with a target dir of this worktree's own.",
            running.display()
        )),
        None => Err(format!("not writing pinned files into {built}: CARGO_MANIFEST_DIR isn't set. Run it through `cargo test`.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_writes_only_into_the_folder_it_runs_from() {
        let built = env!("CARGO_MANIFEST_DIR");
        assert!(check(built, Some(Path::new(built))).is_ok());
        assert!(check(built, Some(&Path::new(built).join("src/.."))).is_ok(), "the same folder, written another way");
        let other = check(built, Some(Path::new("/elsewhere/wt-b/crates/core"))).unwrap_err();
        assert!(other.contains(built) && other.contains("/elsewhere/wt-b/crates/core"), "{other}");
        assert!(check(built, None).unwrap_err().contains("cargo test"));
        here(built);
    }
}
