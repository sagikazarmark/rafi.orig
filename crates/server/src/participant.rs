//! The participant pages' HTTP side: the participant page (`/j/{code}`,
//! and joining by posting its form), the rules and privacy pages, the
//! installation's own documents (`/privacy`, `/terms`), and the
//! proof-of-work solver the page runs. Which raffle, which entrant and what
//! the join did is decided here; `pages` renders them.

use std::sync::LazyLock;

use axum::{
    Extension, Form, Router,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use rafi_lifecycle::{dates::Date, settings::ErasureDates};
use rafi_store::{EntrantRow, RaffleRow};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    AppState, csp,
    pages::{self, Entrant, Notice, Page, Raffle},
    service,
};

const COOKIE_MAX_AGE: i64 = 3 * 86_400;

/// How many generated names the join form's 🎲 button cycles through.
const NAME_IDEAS: usize = 12;

pub fn routes() -> Router {
    Router::new()
        .route("/j/{code}", get(page).post(submit))
        .route("/rules/{slug}", get(rules))
        .route("/privacy/{slug}", get(privacy))
        .route("/privacy", get(installation_privacy))
        .route("/terms", get(installation_terms))
        .route("/participant/pow-worker.js", get(pow_worker))
}

#[derive(Debug, Default, Deserialize)]
pub struct JoinForm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    rules: Option<String>,
    #[serde(default)]
    proof: String,
}

fn cookie_name(raffle_id: &str) -> String {
    format!("rafi_e_{raffle_id}")
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

/// The public raffle joined with `code`; `None` if there's none (or the
/// read failed).
async fn load(app: &AppState, code: &str) -> Option<RaffleRow> {
    found(service::public::raffle_by_join_code(app, code).await)
}

/// Raffle `slug`, if the public may see it.
async fn load_slug(app: &AppState, slug: &str) -> Option<RaffleRow> {
    found(service::public::raffle(app, slug).await)
}

fn found(raffle: service::Result<RaffleRow>) -> Option<RaffleRow> {
    match raffle {
        Ok(r) => Some(r),
        Err(service::Error::NotFound) => None,
        Err(e) => {
            tracing::error!(error = %e, "loading raffle");
            None
        }
    }
}

/// The entrant this device joined `raffle` as (its cookie).
async fn current_entrant(
    app: &AppState,
    raffle: &RaffleRow,
    headers: &HeaderMap,
) -> Option<EntrantRow> {
    let token = cookie(headers, &cookie_name(&raffle.id))?;
    service::public::entrant(app, &raffle.id, &token)
        .await
        .ok()
        .flatten()
}

/// What the pages show of `raffle`, by `app`'s clock.
fn raffle_of<'a>(app: &AppState, raffle: &'a RaffleRow) -> Raffle<'a> {
    Raffle {
        id: &raffle.id,
        title: &raffle.title,
        slug: &raffle.slug,
        join_code: &raffle.join_code,
        test_mode: raffle.test_mode,
        joinable: rafi_lifecycle::entrant::may_join(
            raffle.state,
            raffle.past_erase_date(app.clock.now()),
            true,
        )
        .is_ok(),
    }
}

fn organizer_of(raffle: &RaffleRow) -> pages::Organizer<'_> {
    pages::Organizer {
        name: &raffle.organizer_name,
        contact_email: &raffle.contact_email,
        privacy_contact: raffle.privacy_contact(),
        privacy_text: &raffle.privacy_text,
        privacy_url: &raffle.privacy_url,
        left: raffle.organizer_left_at.is_some(),
    }
}

fn operator_of(app: &AppState) -> pages::Operator<'_> {
    pages::Operator {
        name: &app.cfg.operator.name,
        email: &app.cfg.operator.email,
        privacy_version: &app.cfg.privacy_version,
    }
}

/// When `raffle`'s personal data is erased, as the pages say it. A stored
/// date always parses; the start of time if not, which the page would make
/// obvious.
fn retention_of(app: &AppState, raffle: &RaffleRow) -> ErasureDates {
    let hold = app.policy.retention.winner_hold_days;
    ErasureDates::of(&raffle.event_date, raffle.retention_days, hold).unwrap_or_else(|| {
        let event = Date::from_days(0);
        ErasureDates {
            event,
            erase: event,
            winners_erase: event,
        }
    })
}

/// The participant page of `raffle` for this request.
fn render_page(
    app: &AppState,
    raffle: &RaffleRow,
    live: &service::public::Participant,
    entrant: Option<&EntrantRow>,
    notice: Notice,
    form: &JoinForm,
) -> String {
    pages::page(Page {
        raffle: raffle_of(app, raffle),
        view: &live.view,
        follow: live.follow.clone(),
        entrant: entrant.map(|e| Entrant {
            id: &e.id,
            display_name: &e.display_name,
            claim_code: e.claim_code.as_deref().unwrap_or_default(),
        }),
        notice,
        name: &form.name,
        email: &form.email,
        name_ideas: &service::name_ideas(NAME_IDEAS),
        now_ms: rafi_store::now_ms(),
        retention: retention_of(app, raffle),
        min_age: app.cfg.min_age,
    })
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Html(pages::not_found())).into_response()
}

/// What phones see of `raffle`; `None` if it's gone (or the read failed).
async fn participant(app: &AppState, raffle: &RaffleRow) -> Option<service::public::Participant> {
    match service::public::participant(app, &raffle.id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "reading the participant view");
            None
        }
    }
}

async fn page(
    Extension(app): Extension<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(raffle) = load(&app, &code).await else {
        return not_found();
    };
    if raffle.join_code != code {
        return Redirect::to(&format!("/j/{}", raffle.join_code)).into_response();
    }
    let entrant = current_entrant(&app, &raffle, &headers).await;
    let Some(live) = participant(&app, &raffle).await else {
        return not_found();
    };
    let html = render_page(
        &app,
        &raffle,
        &live,
        entrant.as_ref(),
        Notice::None,
        &JoinForm::default(),
    );
    ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response()
}

async fn submit(
    Extension(app): Extension<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    Form(form): Form<JoinForm>,
) -> Response {
    let Some(raffle) = load(&app, &code).await else {
        return not_found();
    };

    // Already joined on this device: just show the page.
    if current_entrant(&app, &raffle, &headers).await.is_some() {
        return Redirect::to(&format!("/j/{}", raffle.join_code)).into_response();
    }

    let (notice, status) = match try_join(&app, &raffle, &form).await {
        Ok(token) => {
            let secure = if app.cfg.cookie_secure {
                "; Secure"
            } else {
                ""
            };
            // Only the join page reads the cookie (`/j/<code>`), so only
            // requests there carry it: never the APIs, never the live
            // channel's upgrade (whose headers the Worker forwards to the
            // raffle's object), never a proxy's log of them.
            let cookie = format!(
                "{}={token}; Path=/j/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Lax{secure}",
                cookie_name(&raffle.id)
            );
            let mut resp = Redirect::to(&format!("/j/{}", raffle.join_code)).into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().append(header::SET_COOKIE, v);
            }
            return resp;
        }
        Err(refused) => refused,
    };
    let Some(live) = participant(&app, &raffle).await else {
        return not_found();
    };
    let form = if notice == Notice::AlreadyIn {
        JoinForm::default()
    } else {
        form
    };
    let html = render_page(&app, &raffle, &live, None, notice, &form);
    (status, [(header::CACHE_CONTROL, "no-store")], Html(html)).into_response()
}

/// Joins `raffle` with the posted form. Returns the entrant's cookie token,
/// or why (and with what status) the join was refused.
async fn try_join(
    app: &AppState,
    raffle: &RaffleRow,
    form: &JoinForm,
) -> Result<String, (Notice, StatusCode)> {
    let req = service::JoinRequest {
        name: &form.name,
        email: &form.email,
        rules: form.rules.is_some(),
        proof: &form.proof,
    };
    match service::join(app, raffle, &req).await {
        Ok(service::JoinOutcome::Joined { token }) => Ok(token),
        Ok(service::JoinOutcome::AlreadyIn) => Err((Notice::AlreadyIn, StatusCode::OK)),
        Err(e) => Err((Notice::Error(join_error(e)), join_status(e))),
    }
}

/// Why a join was refused, as the page says it.
fn join_error(e: service::JoinError) -> &'static str {
    use rafi_lifecycle::entrant::NameError;
    use service::JoinError as E;
    match e {
        E::Closed => "Registration is closed.",
        E::Name(NameError::Invalid) => {
            "Please choose a name of at most 24 characters, or leave it blank for a generated one."
        }
        E::Name(NameError::Blocked) => "Please choose a different name.",
        E::Email => "Please enter a valid email address.",
        E::Rules => "You need to accept the rules to take part.",
        E::Full => "Sorry, this raffle is full: it has reached its maximum number of entrants.",
        E::Limited | E::Failed => "Too many attempts. Please wait a minute.",
        E::Proof => "Could not verify that you are human. Please reload the page and try again.",
    }
}

fn join_status(e: service::JoinError) -> StatusCode {
    use service::JoinError as E;
    match e {
        E::Limited => StatusCode::TOO_MANY_REQUESTS,
        E::Failed => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

async fn rules(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let Some(r) = load_slug(&app, &slug).await else {
        return not_found();
    };
    let prizes = service::public::prizes(&app, &r.id)
        .await
        .unwrap_or_default();
    let rules = pages::Rules {
        raffle: raffle_of(&app, &r),
        organizer: organizer_of(&r),
        prizes: prizes
            .iter()
            .map(|i| pages::PrizeLine {
                qty: i.qty,
                name: &i.name,
                sponsor: &i.sponsor,
            })
            .collect(),
        additional: &r.rules,
        min_age: app.cfg.min_age,
    };
    Html(pages::rules(&rules)).into_response()
}

async fn privacy(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let Some(r) = load_slug(&app, &slug).await else {
        return not_found();
    };
    let notice = pages::Privacy {
        raffle: raffle_of(&app, &r),
        organizer: organizer_of(&r),
        operator: operator_of(&app),
        retention: retention_of(&app, &r),
    };
    Html(pages::privacy(&notice)).into_response()
}

/// The installation's own privacy notice: the operator's, naming its
/// subprocessors. Under the documents' policy, which lets the operator's
/// Markdown show images from elsewhere.
async fn installation_privacy(Extension(app): Extension<AppState>) -> Response {
    (
        Extension(csp::Page::Legal),
        Html(pages::legal(
            "Privacy notice",
            &app.cfg.privacy_version,
            &app.cfg.documents.privacy,
        )),
    )
        .into_response()
}

/// The operator's terms, with the data processing agreement: what organizers
/// accept in the admin.
async fn installation_terms(Extension(app): Extension<AppState>) -> Response {
    (
        Extension(csp::Page::Legal),
        Html(pages::legal(
            "Terms",
            &app.cfg.terms_version,
            &app.cfg.documents.terms,
        )),
    )
        .into_response()
}

/// [`pow_worker`]'s entity tag: a hash of the script, so it changes when
/// `dioxus-altcha` does.
static POW_WORKER_ETAG: LazyLock<String> = LazyLock::new(|| {
    let hash = Sha256::digest(dioxus_altcha::WORKER_JS.as_bytes());
    format!("\"{}\"", hex::encode(&hash[..8]))
});

/// dioxus-altcha's solver ([`dioxus_altcha::WORKER_JS`]), always the one this
/// server was built with. The participant page's WebAssembly and
/// `fallback.js` start their workers from it, or load it into the page when
/// workers fail. Its URL isn't versioned, so caches revalidate it each time.
async fn pow_worker(headers: HeaderMap) -> Response {
    let etag = POW_WORKER_ETAG.as_str();
    let cache = [(header::ETAG, etag), (header::CACHE_CONTROL, "no-cache")];
    if not_modified(&headers, etag) {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (
        cache,
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        dioxus_altcha::WORKER_JS,
    )
        .into_response()
}

/// Whether `If-None-Match` names `etag` (a weak comparison) or is `*`.
pub(crate) fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}
