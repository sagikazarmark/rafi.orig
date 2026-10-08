//! The backend for the target: SQLite (native) or D1 (Workers).

#[cfg(target_arch = "wasm32")]
mod d1;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub(crate) use d1::D1 as Backend;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Native as Backend;
#[cfg(not(target_arch = "wasm32"))]
pub use native::options;
