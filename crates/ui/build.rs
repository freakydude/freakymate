fn main() {
    // Slint 1.19 offers a newer edition (size of `if`/`for` elements); the UI is not migrated yet, so pin the
    // behaviour of 1.18. See the backlog entry "Slint edition 1.19".
    let config = slint_build::CompilerConfiguration::new().with_edition((1, 18));
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI must compile");
}
