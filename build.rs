// Links the precompiled icon resource (assets/condor.res) into Windows builds, so no
// resource compiler is needed to build.
fn main() {
    println!("cargo:rerun-if-changed=assets/condor.res");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") && target.contains("msvc") {
        let res = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("condor.res");
        println!("cargo:rustc-link-arg-bins={}", res.display());
    }
}
