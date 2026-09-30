//! Generate Swift/Kotlin bindings from a compiled UniFFI library.
//! This tool must not depend on deskfuse-engine, collaboration, or product extensions.
fn main() {
    uniffi::uniffi_bindgen_main();
}
