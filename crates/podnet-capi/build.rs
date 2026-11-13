// The link argument of the shared library (T-304): the soname is
// libpodnet.so (the file is libpodnet_capi.so; docs/build.md).
//
// No version script is passed here. For a cdylib, rustc writes its own
// version script: it exports only the `#[no_mangle]` items of this crate and
// hides each other symbol. A second `--version-script` conflicts with it
// (rust-lld: "attempt to reassign symbol"). tests/c_abi.rs checks that the
// dynamic symbols equal the list of include/podnet.h.
fn main() {
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libpodnet.so");
}
