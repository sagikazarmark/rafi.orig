//! Organizer authentication.

use dioxus::prelude::ServerFnError;

use super::AppState;

/// The owner key for the current organizer: the active Clerk organization if
/// any (co-hosts share raffles), otherwise the Clerk user.
///
/// `ClerkAuthLayer` does not reject requests by itself, so every admin server
/// function must call this.
pub fn owner(app: &AppState) -> Result<String, ServerFnError> {
    if app.cfg.dev_auth {
        return Ok("dev".into());
    }
    let auth =
        dioxus_clerk::server::current_auth().map_err(|_| ServerFnError::new("Please sign in."))?;
    Ok(auth.org_id.unwrap_or(auth.user_id))
}
