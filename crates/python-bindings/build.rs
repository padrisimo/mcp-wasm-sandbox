//! Linker flags for the Python extension module.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // The Python symbols are resolved by the interpreter that loads the module,
        // so they must stay undefined at link time. The same flag covers the test
        // harness, which links the extension module symbols too.
        println!("cargo:rustc-link-arg=-undefined");
        println!("cargo:rustc-link-arg=dynamic_lookup");
    }
}
