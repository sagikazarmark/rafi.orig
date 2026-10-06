//! Rafi: live, verifiable meetup raffles.

// Shared modules are compiled for both the server and the browser; some items
// are only used on one side.
#![cfg_attr(not(feature = "server"), allow(dead_code))]

mod api;
// Installed from the dioxus-daisyui-components registry (`dx components add`); not every
// export is used.
#[allow(unused_imports, dead_code)]
mod components;
mod i18n;
mod icons;
mod model;
#[cfg(feature = "server")]
mod server;
mod ui;

fn main() {
    #[cfg(feature = "server")]
    {
        use std::sync::OnceLock;

        // `serve` may call the closure again on hot reload; build state once.
        static STATE: OnceLock<server::AppState> = OnceLock::new();

        dioxus::serve(|| async move {
            let app = match STATE.get() {
                Some(app) => app.clone(),
                None => {
                    let cfg = server::config::Config::from_env();
                    if cfg.dev_auth {
                        tracing::warn!("RAFI_DEV_AUTH is on: admin pages are NOT protected");
                    }
                    if cfg.drand.mock {
                        tracing::warn!(
                            "RAFI_DRAND_MOCK is on: draws use a public test chain and are NOT verifiable as drand"
                        );
                    }
                    let app = server::AppState::new(cfg).await?;
                    server::spawn_background(&app);
                    STATE.get_or_init(|| app).clone()
                }
            };
            let auth = if app.cfg.dev_auth {
                None
            } else {
                Some(dioxus_clerk::server::ClerkAuthLayer::from_env()?)
            };
            Ok(server::build_router(app, auth))
        });
    }

    #[cfg(not(feature = "server"))]
    dioxus::launch(ui::App);
}
