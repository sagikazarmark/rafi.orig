//! The stylesheet, shared by the web app and the server-rendered participant pages.
//!
//! The server is built with plain `cargo`, so it can't resolve the web bundle's hashed
//! asset names. The stylesheet is bundled without a hash (see rafi-web's `ui::TAILWIND_CSS`) and
//! both sides link it at this stable path.

/// Where the web bundle (and `dx serve`) serves the compiled Tailwind stylesheet.
pub const STYLESHEET_URL: &str = "/assets/tailwind.css";
