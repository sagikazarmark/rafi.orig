//! Rafi: live, verifiable meetup raffles.
//!
//! One crate, built per feature: `web` is the admin, projector and
//! verification page (a Dioxus single-page app, `dx bundle --platform web`);
//! `server` is the native HTTP server (attendee pages, public and admin JSON
//! APIs, and the web app's files). The binary (`main.rs`) only holds the entry
//! points, so other targets (Cloudflare Workers) can build this library.

// Shared modules are compiled for both the server and the browser; some items
// are only used on one side.
#![cfg_attr(not(feature = "server"), allow(dead_code))]

#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub mod api;
// Installed from the dioxus-daisyui-components registry (`dx components add`); not every
// export is used.
#[cfg(feature = "web")]
#[allow(unused_imports, dead_code)]
mod components;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub mod i18n;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub mod icons;
#[cfg_attr(not(feature = "web"), allow(dead_code))]
pub mod model;
#[cfg(feature = "server")]
pub mod server;
pub mod style;
#[cfg(feature = "web")]
pub mod ui;
