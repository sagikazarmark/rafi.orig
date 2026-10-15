//! dioxus-altcha end to end, as a Dioxus fullstack app: `dx serve` here.
//!
//! - `/`: a form protected by the proof-of-work. The server issues the
//!   challenges and verifies the solutions (server functions, src/server.rs).
//! - `/strict`: the same under `Content-Security-Policy: worker-src 'self'`,
//!   which blocks `blob:` workers; the workers load from `/altcha-worker.js`.
//! - `/bench`: measurements on whatever device opens it (src/bench.rs).

mod bench;
mod demo;
mod server;

use dioxus::prelude::*;

const CSS: Asset = asset!("/assets/main.css");

#[derive(Clone, Debug, PartialEq, Routable)]
enum Route {
    #[layout(Layout)]
    #[route("/")]
    Home {},
    #[route("/strict")]
    Strict {},
    #[route("/bench")]
    Bench {},
}

fn main() {
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);

    #[cfg(feature = "server")]
    dioxus::serve(|| async move { Ok(serve::router()) });
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: CSS }
        document::Title { "dioxus-altcha" }
        Router::<Route> {}
    }
}

#[component]
fn Layout() -> Element {
    let route = use_route::<Route>();
    // Plain links, not the router's: each page is a full load, so the server's
    // headers (the strict page's CSP) apply.
    let link = |to: Route, text: &'static str| {
        let class = if route == to { "active" } else { "" };
        rsx! {
            a { class, href: "{to}", "{text}" }
        }
    };
    rsx! {
        header {
            h1 { "dioxus-altcha" }
            nav {
                {link(Route::Home {}, "Demo")}
                {link(Route::Strict {}, "Strict CSP")}
                {link(Route::Bench {}, "Bench")}
            }
        }
        main { Outlet::<Route> {} }
        footer {
            "ALTCHA v2 proof-of-work: the server issues and verifies with the "
            code { "altcha" }
            " crate; the browser solves in Web Workers on WebCrypto."
        }
    }
}

#[component]
fn Home() -> Element {
    rsx! {
        p { class: "lead",
            "The server sends a challenge, the browser finds a counter whose PBKDF2 key starts with "
            code { "00" }
            " (about 256 tries), and the form posts the solution. The server checks it and accepts each challenge once."
        }
        demo::Demo { strict: false }
    }
}

#[component]
fn Strict() -> Element {
    rsx! {
        p { class: "lead",
            "This page is served with "
            code { "Content-Security-Policy: worker-src 'self'" }
            ", so the embedded "
            code { "blob:" }
            " worker can't start. The workers load from "
            code { "/altcha-worker.js" }
            " instead ("
            code { "SolveOptions::worker_url" }
            "), which the server serves from "
            code { "dioxus_altcha::WORKER_JS" }
            ". The policy restricts workers only; with "
            code { "script-src" }
            " restricted too, the embedded fallback couldn't load its "
            code { "blob:" }
            " script either."
        }
        demo::Demo { strict: true }
    }
}

#[component]
fn Bench() -> Element {
    rsx! {
        bench::Bench {}
    }
}

#[cfg(feature = "server")]
mod serve {
    use dioxus::server::axum::{
        Router,
        extract::Request,
        http::{HeaderValue, header},
        middleware::{self, Next},
        response::{IntoResponse, Response},
        routing::get,
    };

    pub fn router() -> Router {
        dioxus::server::router(super::App)
            .route("/altcha-worker.js", get(worker))
            .layer(middleware::from_fn(csp))
    }

    /// The solver, for pages whose CSP forbids `blob:` workers.
    async fn worker() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            dioxus_altcha::WORKER_JS,
        )
    }

    async fn csp(req: Request, next: Next) -> Response {
        let strict = req.uri().path() == "/strict";
        let mut res = next.run(req).await;
        if strict {
            res.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("worker-src 'self'"),
            );
        }
        res
    }
}
