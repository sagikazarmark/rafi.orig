//! Organizer authentication.

use dioxus_clerk::server::ClerkAuth;
use rafi_api::Caller;

use super::AppState;

/// Who is calling: the owner key for the current organizer (the active Clerk
/// organization if any, since co-hosts share raffles, otherwise the Clerk
/// user) and the user acting; both `dev` with dev auth. `None` means the
/// request carries no valid session.
///
/// `ClerkAuthLayer` does not reject requests by itself, so every admin route
/// must call this.
pub fn caller(app: &AppState, auth: Option<ClerkAuth>) -> Option<Caller> {
    if app.cfg.dev_auth {
        return Some(Caller {
            owner: "dev".into(),
            user: "dev".into(),
        });
    }
    auth.map(|auth| Caller {
        owner: auth.org_id.unwrap_or_else(|| auth.user_id.clone()),
        user: auth.user_id,
    })
}
