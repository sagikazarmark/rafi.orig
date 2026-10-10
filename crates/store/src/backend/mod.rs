//! The backend for the target: SQLite or Postgres (native), or D1 (Workers).

#[cfg(target_arch = "wasm32")]
mod d1;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
mod postgres;
#[cfg(not(target_arch = "wasm32"))]
mod sqlite;

#[cfg(target_arch = "wasm32")]
pub(crate) use d1::D1 as Backend;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Native as Backend;
#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
pub use postgres::fresh_schema;
#[cfg(not(target_arch = "wasm32"))]
pub use sqlite::options;
