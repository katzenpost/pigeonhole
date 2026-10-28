use std::path::Path;

fn main() {
    let dir = Path::new("src/highctidh/csrc");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let ptr_width = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap();

    let mut build = cc::Build::new();
    build
        .include(dir)
        .define("BITS", "1024")
        .define("CGONUTS", None)
        .define("HIGHCTIDH_PORTABLE", "1")
        .define("PLATFORM", arch.as_str())
        .define("PLATFORM_SIZE", ptr_width.as_str())
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-ivopts")
        .warnings(false)
        .opt_level(2);
    if matches!(os.as_str(), "linux" | "android" | "macos" | "freebsd") {
        build.define("GETRANDOM", None);
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "c") {
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(path);
        }
    }
    build.compile("highctidh_1024");
}
