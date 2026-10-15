//! `GET` a challenge with the browser's `fetch`: an HTTP client library costs
//! tens of kilobytes for one request.

use std::future::Future;

use crate::RawChallenge;

/// Why [`fetch_challenge`] failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    #[error("the request failed: {0}")]
    Network(String),
    #[error("the server answered {0}")]
    Status(u16),
    #[error("the response isn't a challenge: {0}")]
    InvalidBody(String),
    #[error("fetching isn't supported on this target")]
    UnsupportedTarget,
}

/// `GET`s `url` (uncached) and reads the body as a challenge. In the browser
/// only; elsewhere it fails with [`FetchError::UnsupportedTarget`] (use a
/// server function there).
pub fn fetch_challenge(
    url: impl Into<String>,
) -> impl Future<Output = Result<RawChallenge, FetchError>> + 'static {
    let url = url.into();
    async move { imp::fetch(&url).await }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestCache, RequestInit, Response};

    use super::FetchError;
    use crate::RawChallenge;

    fn network(e: wasm_bindgen::JsValue) -> FetchError {
        FetchError::Network(crate::solve::js_error(&e))
    }

    pub async fn fetch(url: &str) -> Result<RawChallenge, FetchError> {
        let window = web_sys::window().ok_or(FetchError::UnsupportedTarget)?;
        let init = RequestInit::new();
        init.set_cache(RequestCache::NoStore);
        let request = Request::new_with_str_and_init(url, &init).map_err(network)?;
        let response: Response = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(network)?
            .dyn_into()
            .map_err(network)?;
        if !response.ok() {
            return Err(FetchError::Status(response.status()));
        }
        let text = JsFuture::from(response.text().map_err(network)?)
            .await
            .map_err(network)?
            .as_string()
            .unwrap_or_default();
        RawChallenge::from_json(text).map_err(|e| FetchError::InvalidBody(e.to_string()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::FetchError;
    use crate::RawChallenge;

    pub async fn fetch(_url: &str) -> Result<RawChallenge, FetchError> {
        Err(FetchError::UnsupportedTarget)
    }
}
