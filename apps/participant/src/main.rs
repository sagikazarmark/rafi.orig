//! The entry point. dx builds the participant as a binary: its glue calls
//! `main` once the module is instantiated. (As a `cdylib`, every
//! `#[no_mangle]` symbol in the dependency graph would stay alive through
//! LTO, and the module was a quarter larger for it.)

fn main() {
    #[cfg(target_arch = "wasm32")]
    rafi_participant::start();
}
