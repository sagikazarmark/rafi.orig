//! The admin API's browser client ([`rafi_api::admin`]): one function per
//! admin function, generated from [`rafi_api::admin_functions!`], that
//! POSTs its arguments to `/api/admin/<name>` and decodes the reply; and
//! the public reads the web app makes.

pub use rafi_api::admin::ApiError;
use rafi_api::{RaffleView, VerifyData};

macro_rules! client_side {
    ($(
        $(#[$meta:meta])*
        fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty;
    )*) => {
        $(
            $(#[$meta])*
            // The client covers the whole admin API; the UI needn't call every
            // function (it never calls `whoami`).
            #[allow(dead_code)]
            pub async fn $name($($arg: $ty),*) -> Result<$ret, ApiError> {
                #[derive(serde::Serialize)]
                struct Args { $($arg: $ty),* }
                let path = format!("{}{}", rafi_api::admin::PATH, stringify!($name));
                client::post(&path, &Args { $($arg),* }).await
            }
        )*
    };
}

rafi_api::admin_functions!(client_side);

/// Public: data for the verification page (`/api/r/<slug>/verify.json`, no session needed).
pub async fn get_verify(slug: String) -> Result<VerifyData, ApiError> {
    let slug = String::from(js_sys::encode_uri_component(&slug));
    client::get(&format!("/api/r/{slug}/verify.json")).await
}

/// Public: a raffle's full view (`/api/r/<slug>/view`), which the projector and
/// the admin poll.
pub async fn get_view(slug: String) -> Result<RaffleView, ApiError> {
    let slug = String::from(js_sys::encode_uri_component(&slug));
    client::get(&format!("/api/r/{slug}/view")).await
}

/// Same-origin requests, so the browser sends Clerk's session cookie.
mod client {
    use gloo_net::http::{Request, Response};
    use serde::{Serialize, de::DeserializeOwned};

    use super::ApiError;

    pub async fn post<A: Serialize, T: DeserializeOwned>(
        url: &str,
        args: &A,
    ) -> Result<T, ApiError> {
        let req = Request::post(url).json(args).map_err(network)?;
        reply(req.send().await.map_err(network)?).await
    }

    pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, ApiError> {
        reply(Request::get(url).send().await.map_err(network)?).await
    }

    fn network(e: gloo_net::Error) -> ApiError {
        ApiError::new(format!("Could not reach the server: {e}"))
    }

    async fn reply<T: DeserializeOwned>(resp: Response) -> Result<T, ApiError> {
        let body = resp.text().await.map_err(network)?;
        if resp.ok() {
            return serde_json::from_str(&body)
                .map_err(|e| ApiError::new(format!("Unexpected reply from the server: {e}")));
        }
        if let Ok(e) = serde_json::from_str::<ApiError>(&body) {
            return Err(e);
        }
        Err(ApiError::new(match resp.status() {
            401 => "Please sign in.".to_owned(),
            404 => "not found".to_owned(),
            status if body.trim().is_empty() => format!("Request failed ({status})."),
            _ => body,
        }))
    }
}
