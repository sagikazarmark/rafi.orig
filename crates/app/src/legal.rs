//! The operator's terms, with the data processing agreement: organizers
//! accept the current version before they create raffles.

use rafi_store::legal;

use crate::{App, Refusal, Result};

/// Whether `owner` accepted the current terms.
pub async fn terms_accepted(app: &App, owner: &str) -> Result<bool> {
    Ok(legal::accepted(&app.db, owner, legal::TERMS, &app.policy.terms_version).await?)
}

/// Refuses unless `owner` accepted the current terms.
pub(crate) async fn require_terms(app: &App, owner: &str) -> Result<()> {
    if terms_accepted(app, owner).await? {
        Ok(())
    } else {
        Err(Refusal::TermsNotAccepted.into())
    }
}

/// Records that `user`, for `owner`, accepts `version` of the terms: the
/// current one, or the acceptance is refused (the organizer read an older
/// text). The digest of the current text goes with it, as evidence of what
/// was accepted should the text change under the same version label.
pub async fn accept_terms(app: &App, owner: &str, user: &str, version: &str) -> Result<()> {
    if version != app.policy.terms_version {
        return Err(Refusal::TermsOutdated.into());
    }
    legal::record(
        &app.db,
        owner,
        user,
        legal::TERMS,
        version,
        &app.policy.terms_digest,
    )
    .await?;
    Ok(())
}
