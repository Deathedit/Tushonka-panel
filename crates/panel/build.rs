// rust-embed needs the UI folder to exist at compile time, even before the first `npm run build`.
fn main() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    let _ = std::fs::create_dir_all(&dist);
    println!("cargo:rerun-if-changed=../../web/dist");
    // Optional bundled bridge mod DLL (see `make bridge`).
    let bridge = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bridge/dist");
    let _ = std::fs::create_dir_all(&bridge);
    println!("cargo:rerun-if-changed=../../bridge/dist");
}
