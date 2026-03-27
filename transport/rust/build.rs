fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir   = std::path::PathBuf::from(&crate_dir).join("include");
    std::fs::create_dir_all(&out_dir).ok();

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(cbindgen::Config::from_file("cbindgen.toml").unwrap_or_default())
        .generate()
        .expect("cbindgen failed")
        .write_to_file(out_dir.join("borgkit_libp2p.h"));
}
