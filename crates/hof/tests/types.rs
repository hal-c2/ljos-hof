//! `ljostypes.json` says what `hof::register` registers: the build script
//! compiles the QML against it.
//! `LJOS_TYPES=write cargo test -p hof --no-fail-fast` writes it.

#[test]
fn the_type_dump_file_is_current() {
    ljos::types::check_file("ljostypes.json".as_ref(), hof::register).unwrap();
}
