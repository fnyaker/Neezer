//! `uniffi-bindgen`: the generator binary, a one-line wrapper around UniFFI's
//! own CLI so the version that generates the bindings is exactly the version
//! the crate was built with — never a separately installed tool that could
//! drift (`build-android.sh` runs `cargo run -p nz-bindgen`).

fn main() {
    uniffi::uniffi_bindgen_main()
}
