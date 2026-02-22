fn main() {
    // On macOS, embed Info.plist into the binary's __TEXT,__info_plist section.
    // macOS reads LSUIElement (and other keys) from this section even for
    // standalone binaries that are not packaged as a .app bundle.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg=-sectcreate");
        println!("cargo:rustc-link-arg=__TEXT");
        println!("cargo:rustc-link-arg=__info_plist");
        println!("cargo:rustc-link-arg={dir}/Info.plist");
        println!("cargo:rerun-if-changed=Info.plist");
    }
}
