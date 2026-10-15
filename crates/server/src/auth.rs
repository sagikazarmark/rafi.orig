//! Organizer authentication.

use dioxus_clerk::server::ClerkAuth;

use super::AppState;

/// The owner key for the current organizer: the active Clerk organization if
/// any (co-hosts share raffles), otherwise the Clerk user; `dev` with dev auth.
/// `None` means the request carries no valid session.
///
/// `ClerkAuthLayer` does not reject requests by itself, so every admin route
/// must call this.
pub fn owner(app: &AppState, auth: Option<ClerkAuth>) -> Option<String> {
    if app.cfg.dev_auth {
        return Some("dev".into());
    }
    auth.map(|auth| auth.org_id.unwrap_or(auth.user_id))
}
