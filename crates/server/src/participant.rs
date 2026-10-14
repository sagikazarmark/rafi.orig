//! The participant pages' HTTP side: the participant page (`/j/{code}`,
//! and joining by posting its form), the device's own entry, and the rules
//! and privacy pages. Which language, which raffle, which entrant and what
//! the join did is decided here; [`rafi_pages`] renders them.

use axum::{
    Extension, Form, Json, Router,
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use rafi_pages::{
    i18n::Lang,
    participant::{self as pages, Entrant, Notice, Page, Raffle},
};
use rafi_store::{EntrantRow, RaffleRow};
use serde::Deserialize;

use super::{AppState, service};

const COOKIE_MAX_AGE: i64 = 3 * 86_400;

pub fn routes() -> Router {
    Router::new()
        .route("/j/{code}", get(page).post(submit))
        .route("/api/j/{code}/me", get(me))
        .route("/rules/{slug}", get(rules))
        .route("/privacy/{slug}", get(privacy))
}

#[derive(Debug, Default, Deserialize)]
pub struct LangQuery {
    lang: Option<String>,
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
    marketing: Option<String>,
    #[serde(default)]
    altcha: String,
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

/// Language: `?lang=`, then the `rafi_lang` cookie, then `Accept-Language`, then the raffle default.
fn pick_lang(query: &LangQuery, headers: &HeaderMap, raffle: Option<&RaffleRow>) -> Lang {
    if let Some(l) = &query.lang {
        return Lang::parse(l);
    }
    if let Some(l) = cookie(headers, "rafi_lang") {
        return Lang::parse(&l);
    }
    let accept = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if accept.to_ascii_lowercase().starts_with("hu") {
        return Lang::Hu;
    }
    raffle.map(|r| Lang::parse(&r.lang)).unwrap_or(Lang::En)
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

/// What the pages show of `raffle`.
fn raffle_of(raffle: &RaffleRow) -> Raffle<'_> {
    Raffle {
        id: &raffle.id,
        title: &raffle.title,
        slug: &raffle.slug,
        join_code: &raffle.join_code,
        test_mode: raffle.test_mode,
    }
}

fn organizer_of(raffle: &RaffleRow) -> rafi_pages::Organizer<'_> {
    rafi_pages::Organizer {
        name: &raffle.organizer_name,
        contact_email: &raffle.contact_email,
    }
}

/// The participant page of `raffle` for this request.
fn render_page(
    raffle: &RaffleRow,
    live: &service::public::Participant,
    lang: Lang,
    entrant: Option<&EntrantRow>,
    notice: Notice,
    form: &JoinForm,
) -> String {
    pages::page(Page {
        raffle: raffle_of(raffle),
        view: &live.view,
        follow: live.follow.clone(),
        lang,
        entrant: entrant.map(|e| Entrant {
            id: &e.id,
            display_name: &e.display_name,
            claim_code: &e.claim_code,
        }),
        notice,
        name: &form.name,
        email: &form.email,
        now_ms: rafi_model::now_unix_ms(),
    })
}

fn not_found(lang: Lang) -> Response {
    (StatusCode::NOT_FOUND, Html(pages::not_found(lang))).into_response()
}

fn with_lang_cookie(mut resp: Response, query: &LangQuery, secure: bool) -> Response {
    if let Some(l) = &query.lang {
        let lang = Lang::parse(l);
        let secure = if secure { "; Secure" } else { "" };
        if let Ok(v) = HeaderValue::from_str(&format!(
            "rafi_lang={}; Path=/; Max-Age=31536000; SameSite=Lax{secure}",
            lang.code()
        )) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
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
    Query(q): Query<LangQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(raffle) = load(&app, &code).await else {
        return not_found(pick_lang(&q, &headers, None));
    };
    if raffle.join_code != code {
        return Redirect::to(&format!("/j/{}", raffle.join_code)).into_response();
    }
    let lang = pick_lang(&q, &headers, Some(&raffle));
    let entrant = current_entrant(&app, &raffle, &headers).await;
    let Some(live) = participant(&app, &raffle).await else {
        return not_found(lang);
    };
    let html = render_page(
        &raffle,
        &live,
        lang,
        entrant.as_ref(),
        Notice::None,
        &JoinForm::default(),
    );
    let resp = ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response();
    with_lang_cookie(resp, &q, app.cfg.cookie_secure)
}

async fn submit(
    Extension(app): Extension<AppState>,
    Path(code): Path<String>,
    Query(q): Query<LangQuery>,
    headers: HeaderMap,
    Form(form): Form<JoinForm>,
) -> Response {
    let Some(raffle) = load(&app, &code).await else {
        return not_found(pick_lang(&q, &headers, None));
    };
    let lang = pick_lang(&q, &headers, Some(&raffle));

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
            let cookie = format!(
                "{}={token}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Lax{secure}",
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
        return not_found(lang);
    };
    let form = if notice == Notice::AlreadyIn {
        JoinForm::default()
    } else {
        form
    };
    let html = render_page(&raffle, &live, lang, None, notice, &form);
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
        marketing: form.marketing.is_some(),
        proof: &form.altcha,
    };
    match service::join(app, raffle, &req).await {
        Ok(service::JoinOutcome::Joined { token }) => Ok(token),
        Ok(service::JoinOutcome::AlreadyIn) => Err((Notice::AlreadyIn, StatusCode::OK)),
        Err(e) => Err((Notice::Error(join_error(e)), join_status(e))),
    }
}

/// The i18n key of why a join was refused.
fn join_error(e: service::JoinError) -> &'static str {
    use rafi_lifecycle::entrant::NameError;
    use service::JoinError as E;
    match e {
        E::Closed => "reg_closed",
        E::Name(NameError::Invalid) => "err_name",
        E::Name(NameError::Blocked) => "err_name_blocked",
        E::Email => "err_email",
        E::Rules => "err_rules",
        E::Full => "err_full",
        E::Limited | E::Failed => "err_rate",
        E::Proof => "err_bot",
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

/// The current participant's own entry (cookie-authenticated, never cached).
async fn me(
    Extension(app): Extension<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(raffle) = load(&app, &code).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let body = match current_entrant(&app, &raffle, &headers).await {
        Some(e) => {
            serde_json::json!({ "entrant_id": e.id, "display_name": e.display_name, "claim_code": e.claim_code })
        }
        None => serde_json::Value::Null,
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

async fn rules(
    Extension(app): Extension<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<LangQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(r) = load_slug(&app, &slug).await else {
        return not_found(pick_lang(&q, &headers, None));
    };
    let lang = pick_lang(&q, &headers, Some(&r));
    let prizes = service::public::prizes(&app, &r.id)
        .await
        .unwrap_or_default();
    let rules = rafi_pages::Rules {
        raffle: raffle_of(&r),
        organizer: organizer_of(&r),
        min_age: r.min_age,
        prizes: prizes
            .iter()
            .map(|i| rafi_pages::PrizeLine {
                qty: i.qty,
                name: &i.name,
                sponsor: &i.sponsor,
            })
            .collect(),
        additional: &r.rules,
    };
    Html(rafi_pages::rules(lang, &rules)).into_response()
}

async fn privacy(
    Extension(app): Extension<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<LangQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(r) = load_slug(&app, &slug).await else {
        return not_found(pick_lang(&q, &headers, None));
    };
    let lang = pick_lang(&q, &headers, Some(&r));
    let retention = app.policy.retention;
    Html(rafi_pages::privacy(
        lang,
        raffle_of(&r),
        organizer_of(&r),
        retention.nonwinners_days,
        retention.winners_days,
    ))
    .into_response()
}
