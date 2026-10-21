//! Organizer authentication and what the session may do.

use dioxus_clerk::server::ClerkAuth;
use rafi_api::{Access, Caller};

use super::AppState;

/// Who is calling: the owner key for the current organizer (the active Clerk
/// organization if any, since co-hosts share raffles, otherwise the Clerk
/// user), the user acting, and their tier ([`Access`]: everything for a
/// personal account, the organization's role or permission otherwise); all
/// `dev` with every right under dev auth. `None` means the request carries
/// no valid session.
///
/// `ClerkAuthLayer` does not reject requests by itself, so every admin route
/// must call this.
pub fn caller(app: &AppState, auth: Option<ClerkAuth>) -> Option<Caller> {
    if app.cfg.dev_auth {
        return Some(Caller {
            owner: "dev".into(),
            user: "dev".into(),
            access: Access::Admin,
        });
    }
    auth.map(|auth| match auth.org_id {
        Some(org) => Caller {
            owner: org,
            user: auth.user_id,
            access: Access::of_organization(auth.org_role.as_deref(), &auth.org_permissions),
        },
        None => Caller {
            owner: auth.user_id.clone(),
            user: auth.user_id,
            access: Access::Admin,
        },
    })
}
