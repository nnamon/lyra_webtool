//! Rebuild when the build stamp changes, so Settings never shows a stale
//! version string from a cached compilation.

fn main() {
    println!("cargo:rerun-if-env-changed=LYRA_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=LYRA_BUILD_DATE");
}
