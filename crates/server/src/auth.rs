//! Organizer authentication and what the session may do.

use dioxus_clerk::server::{ClerkAuth, ClerkAuthLayer, ClerkAuthLayerConfig};
use rafi_api::{Access, Caller};

use super::{AppState, config::Config};

/// Clerk's layer as both hosts build it: `config` (the secret key, and on
/// Workers the JWKS transport) plus what the app knows, the public origin as
/// the one authorized party. A session token names the origin that asked
/// for it (`azp`); one minted for another site on the same Clerk instance is
/// refused, as Clerk recommends. Tokens without `azp` still pass (Clerk's
/// own guidance), so this is defence in depth, not the only check.
pub fn clerk_layer(
    cfg: &Config,
    config: ClerkAuthLayerConfig,
) -> Result<ClerkAuthLayer, dioxus_clerk::core::ClerkError> {
    ClerkAuthLayer::from_config(config.add_authorized_party(cfg.public_url.clone()))
}

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
