//! Configuration from a Worker's vars and secrets, and its database.

use rafi_store::Db;
use worker::{Env, Result};

/// The D1 binding of rafi's database (the app, the draw engine and the
/// channel objects all read and write it).
pub const D1_BINDING: &str = "DB";

/// A setting: the secret `name`, else the var `name`. Empty counts as unset.
///
/// `std::env` always fails on Workers, so configuration must come from here.
pub fn lookup(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|s| s.to_string())
        .ok()
        .or_else(|| env.var(name).map(|v| v.to_string()).ok())
        .filter(|v| !v.trim().is_empty())
}

/// Rafi's database, from the [`D1_BINDING`].
pub fn database(env: &Env) -> Result<Db> {
    Ok(Db::d1(env.d1(D1_BINDING)?))
}
