//! What the server tells the web app before it starts: `index.html` loads
//! `/config.js` (rafi-server's `public::config_js`) ahead of the app's own
//! script, and it sets `window.__RAFI_CONFIG__`. Nothing of an
//! installation's is compiled into the web app, so one bundle serves every
//! installation, and the server alone decides how organizers sign in.

use wasm_bindgen::JsValue;

/// The global `/config.js` sets.
const GLOBAL: &str = "__RAFI_CONFIG__";

/// Clerk's publishable key, when the server verifies Clerk sessions. `None`
/// under dev auth (the server said so), and when `/config.js` didn't load
/// (an old bundle against a newer server, or the reverse): the admin then
/// shows the dev-auth badge rather than a sign-in the server wouldn't check.
pub fn clerk_publishable_key() -> Option<String> {
    string(&field(&config()?, "clerkPublishableKey")?)
}

/// Which Rafi the server is, and where its source and its verifier are:
/// what the footers show and link. `None` from a server before them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Build {
    pub version: Option<String>,
    pub source_url: Option<String>,
    pub verifier_url: Option<String>,
    pub guide_url: Option<String>,
}

pub fn build() -> Build {
    let Some(cfg) = config() else {
        return Build::default();
    };
    let get = |name| field(&cfg, name).as_ref().and_then(string);
    Build {
        version: get("version"),
        source_url: get("sourceUrl"),
        verifier_url: get("verifierUrl"),
        guide_url: get("guideUrl"),
    }
}

/// Which of the operator's documents there are: their terms (`/terms`),
/// which organizers accept by using the service, and their privacy notice
/// (`/privacy`). Neither when the server didn't say (natively, or from a
/// server before them): nothing then claims or links a document that may
/// not be there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Documents {
    pub terms: bool,
    pub privacy: bool,
}

pub fn documents() -> Documents {
    let Some(cfg) = config() else {
        return Documents::default();
    };
    let flag = |name| field(&cfg, name).and_then(|v| v.as_bool()) == Some(true);
    Documents {
        terms: flag("terms"),
        privacy: flag("privacy"),
    }
}

fn config() -> Option<JsValue> {
    // Natively (the tests' server-side rendering) there is no page to ask.
    if !cfg!(target_arch = "wasm32") {
        return None;
    }
    field(&js_sys::global(), GLOBAL)
}

/// `object[name]`, unless it's missing, `null` or `undefined`.
fn field(object: &JsValue, name: &str) -> Option<JsValue> {
    js_sys::Reflect::get(object, &JsValue::from_str(name))
        .ok()
        .filter(|v| !v.is_null() && !v.is_undefined())
}

/// A non-empty string.
fn string(value: &JsValue) -> Option<String> {
    value.as_string().filter(|s| !s.is_empty())
}
