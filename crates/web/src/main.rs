//! Rafi's web app (`dx bundle --platform web`): the organizer's admin, the
//! projector screen and the verification page, a Dioxus single-page app.
//! It talks to the server through [`rafi_model`]: the views it polls and
//! the admin API ([`api`]).

mod api;
// Installed from the dioxus-daisyui-components registry (`dx components add`); not every
// export is used.
#[allow(unused_imports, dead_code)]
mod components;
mod ui;

use rafi_model as model;

fn main() {
    dioxus::launch(ui::App);
}
