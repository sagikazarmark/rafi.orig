//! Server functions called by the Dioxus UI.
//!
//! Every admin function resolves the organizer with `auth::owner`, which
//! fails unless the request carries a valid Clerk session.

use dioxus::prelude::*;
use rafi_core::state::Outcome;

use crate::model::{
    EntrantPatch, ItemInput, Lifecycle, Me, RaffleAdmin, RaffleSettings, RaffleSummary, VerifyData,
};
#[cfg(feature = "server")]
use crate::server::{self, auth, engine::Command, service};

#[cfg(feature = "server")]
fn ctx() -> Result<(server::AppState, String), ServerFnError> {
    let app = server::current()?;
    let owner = auth::owner(&app)?;
    Ok((app, owner))
}

#[post("/api/admin/whoami")]
pub async fn whoami() -> Result<Me, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(Me {
        owner_id: owner,
        dev_auth: app.cfg.dev_auth,
    })
}

#[server]
pub async fn list_raffles() -> Result<Vec<RaffleSummary>, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::list(&app, &owner).await?)
}

#[server]
pub async fn create_raffle(title: String) -> Result<String, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::create(&app, &owner, &title).await?)
}

#[server]
pub async fn get_raffle(id: String) -> Result<RaffleAdmin, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::admin(&app, &owner, &id).await?)
}

#[server]
pub async fn save_settings(id: String, settings: RaffleSettings) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::update_settings(&app, &owner, &id, settings).await?)
}

#[server]
pub async fn delete_raffle(id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::delete(&app, &owner, &id).await?)
}

#[server]
pub async fn duplicate_raffle(id: String) -> Result<String, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::duplicate(&app, &owner, &id).await?)
}

#[server]
pub async fn save_item(id: String, item: ItemInput) -> Result<String, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::upsert_item(&app, &owner, &id, item).await?)
}

#[server]
pub async fn delete_item(id: String, item_id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::delete_item(&app, &owner, &id, &item_id).await?)
}

#[server]
pub async fn lifecycle(id: String, action: Lifecycle) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::lifecycle(&app, &owner, &id, action).await?)
}

#[server]
pub async fn start_draw(id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::start_draw(&app, &owner, &id).await?)
}

#[server]
pub async fn draw_unit(
    id: String,
    unit_id: String,
    offline: bool,
    command_id: String,
) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::command(
        &app,
        &owner,
        &id,
        Command::Draw { unit_id, offline },
        &command_id,
    )
    .await?)
}

#[server]
pub async fn resolve_unit(
    id: String,
    unit_id: String,
    outcome: Outcome,
    command_id: String,
) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::command(
        &app,
        &owner,
        &id,
        Command::Resolve { unit_id, outcome },
        &command_id,
    )
    .await?)
}

#[server]
pub async fn void_unit(
    id: String,
    unit_id: String,
    command_id: String,
) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::command(&app, &owner, &id, Command::Void { unit_id }, &command_id).await?)
}

#[server]
pub async fn finish_raffle(id: String, command_id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::command(&app, &owner, &id, Command::Finish, &command_id).await?)
}

#[server]
pub async fn switch_to_local(id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::switch_to_local(&app, &owner, &id).await?)
}

#[server]
pub async fn add_entrant(
    id: String,
    name: String,
    email: Option<String>,
) -> Result<String, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::add_manual(&app, &owner, &id, &name, email).await?)
}

#[server]
pub async fn add_fake_entrants(id: String, n: u32) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::add_fakes(&app, &owner, &id, n).await?)
}

#[server]
pub async fn patch_entrant(
    id: String,
    entrant_id: String,
    patch: EntrantPatch,
) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::patch_entrant(&app, &owner, &id, &entrant_id, patch).await?)
}

#[server]
pub async fn remove_entrant(id: String, entrant_id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::remove_entrant(&app, &owner, &id, &entrant_id).await?)
}

#[server]
pub async fn export_winners(id: String) -> Result<String, ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::export_csv(&app, &owner, &id).await?)
}

#[server]
pub async fn purge_raffle(id: String) -> Result<(), ServerFnError> {
    let (app, owner) = ctx()?;
    Ok(service::purge(&app, &owner, &id).await?)
}

/// Public: data for the verification page.
#[server]
pub async fn get_verify(slug: String) -> Result<VerifyData, ServerFnError> {
    let app = server::current()?;
    Ok(service::verify_data(&app, &slug).await?)
}
