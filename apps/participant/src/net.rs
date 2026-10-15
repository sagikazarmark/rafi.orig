//! `fetch` through web-sys: a full HTTP client library costs tens of
//! kilobytes on the phone for two GET requests.

use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// GETs `url` and decodes the JSON body. Also returns the `Date` header.
pub async fn get_json<T: DeserializeOwned>(url: &str) -> Option<(T, Option<String>)> {
    let window = web_sys::window()?;
    let res: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !res.ok() {
        return None;
    }
    let date = res.headers().get("date").ok().flatten();
    let text = JsFuture::from(res.text().ok()?).await.ok()?.as_string()?;
    Some((serde_json::from_str(&text).ok()?, date))
}
