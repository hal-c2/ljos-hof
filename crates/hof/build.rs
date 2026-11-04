//! Compiles `qml/` against the native types `ljostypes.json` describes
//! into `$OUT_DIR/qml.pack`, and its bindings and handlers to Rust in
//! `$OUT_DIR/qml.rs`. `tests/types.rs` writes the type file.

fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
    if let Err(e) = ljos::build::compile_dirs_with_types_to_rust(
        &["qml".as_ref()],
        "ljostypes.json".as_ref(),
        &out.join("qml.pack"),
    ) {
        panic!("{e}");
    }
}
