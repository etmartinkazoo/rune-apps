// On macOS the binary carries an Info.plist of its own, with why it uses Bluetooth: without that,
// macOS ends any process that touches Bluetooth, the bundle-less `cargo run` one included.
fn main() {
    println!("cargo:rerun-if-changed=assets/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/Info.plist");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
}
