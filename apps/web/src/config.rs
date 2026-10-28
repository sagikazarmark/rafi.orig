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

fn config() -> Option<JsValue> {
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
