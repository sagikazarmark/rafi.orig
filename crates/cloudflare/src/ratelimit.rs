//! The `RateLimit` over Workers' rate-limiting bindings.

use rafi_platform::{BoxFuture, Limit, RateLimit};
use worker::{
    Env,
    send::{SendFuture, SendWrapper},
};

/// Cloudflare's rate-limiting bindings, one per [`Limit`]: approximate and per
/// location, so a soft limit (the hard one is the entrant cap in the
/// database). A missing binding, or one that fails, allows.
pub struct RateLimitBindings(SendWrapper<Env>);

impl RateLimitBindings {
    pub fn new(env: Env) -> Self {
        Self(SendWrapper(env))
    }

    /// The binding (`wrangler.toml`) that counts `limit`.
    pub fn binding(limit: Limit) -> &'static str {
        match limit {
            Limit::JoinPerRaffle => "JOIN_PER_RAFFLE_LIMITER",
            Limit::JoinPerEmail => "JOIN_PER_EMAIL_LIMITER",
        }
    }
}

impl RateLimit for RateLimitBindings {
    fn allow<'a>(&'a self, limit: Limit, key: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(SendFuture::new(async move {
            let Ok(limiter) = self.0.rate_limiter(Self::binding(limit)) else {
                return true;
            };
            match limiter.limit(key.to_owned()).await {
                Ok(outcome) => outcome.success,
                Err(e) => {
                    worker::console_error!("rafi: rate limiter {}: {e}", Self::binding(limit));
                    true
                }
            }
        }))
    }
}
