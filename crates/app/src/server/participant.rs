//! The participant phone page.
//!
//! The server renders the whole page, including the live card
//! ([`rafi_live_client::Card`], the same component the phone's WebAssembly mounts),
//! and embeds the page's state as JSON in `#rafi-config` (see
//! [`rafi_live_client::PageConfig`]). An inline loader then
//! starts the WebAssembly from `/participant/` (`./scripts/participant.sh`), or the
//! JavaScript fallback when it can't run. They keep the card current, solve
//! the join form's proof-of-work and enable its button.

use axum::{
    Extension, Form, Json, Router,
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use dioxus::prelude::*;
use rafi_live_client::{Card, CardStrings, PageConfig, Phase};
use rafi_store::{EntrantRow, RaffleRow};
use serde::{Deserialize, Serialize};

use super::{AppState, service};
use crate::{i18n::Lang, icons::I};

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

/// Runs before first paint: records when the page started parsing (the
/// phone's clock offset is measured from it) and applies the saved colour
/// scheme (shared with the Dioxus app).
const HEAD_SCRIPT: &str = r#"window.__rafiT0=Date.now();try{var t=localStorage.getItem("rafi-theme");if(t)document.documentElement.setAttribute("data-theme",t)}catch(e){}"#;

/// Starts the participant WebAssembly, or the JavaScript fallback when there is
/// no WebAssembly, the module fails to load, or it isn't ready within 8 s
/// (the module's panic hook also calls `rafiFallback`).
const LOADER: &str = r#"window.rafiFallback=function(){if(window.__rafiFallback)return;window.__rafiFallback=1;var s=document.createElement("script");s.src="/participant/fallback.js";document.body.appendChild(s)};
setTimeout(function(){if(!window.__rafiReady)window.rafiFallback()},8000);
if(typeof WebAssembly!=="object"){window.rafiFallback()}else{import("/participant/rafi_participant.js").then(function(m){return m.default()}).catch(function(e){console.error(e);window.rafiFallback()})}"#;

/// A page: `tail` is raw HTML appended to the body (scripts).
fn document(lang: Lang, title: &str, body: String, tail: Option<String>) -> String {
    let title = escape(title);
    let css = escape(crate::style::STYLESHEET_URL);
    let tail = tail.unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\
         <meta name=\"robots\" content=\"noindex\"><meta name=\"color-scheme\" content=\"light dark\">\
         <script>{HEAD_SCRIPT}</script>\
         <link rel=\"stylesheet\" href=\"{css}\">\
         <title>{title}</title></head><body class=\"min-h-dvh bg-base-200 bg-dots\">{body}{tail}</body></html>",
        lang.code()
    )
}

/// Header shared by the participant pages: logo, language switch, theme toggle.
fn header_bar(lang: Lang, switch_href: &str, test_mode: bool) -> Element {
    let other = lang.other();
    let ticket = I::Ticket.svg("size-4");
    let theme_icons = format!(
        "{}{}{}",
        I::Monitor.svg("size-4 theme-icon-system"),
        I::Sun.svg("size-4 theme-icon-light hidden"),
        I::Moon.svg("size-4 theme-icon-dark hidden")
    );
    rsx! {
        header { class: "flex items-center gap-2",
            span { class: "brand text-lg",
                span { class: "grid place-items-center rounded-lg bg-primary p-1 text-primary-content -rotate-6", dangerous_inner_html: "{ticket}" }
                "rafi"
            }
            if test_mode {
                span { class: "badge badge-warning badge-sm font-bold", "🧪 {lang.t(\"test_mode\")}" }
            }
            span { class: "flex-1" }
            a { class: "btn btn-ghost btn-sm btn-circle font-bold", href: "{switch_href}", hreflang: "{other.code()}", aria_label: "{other.code()}", "{other.code().to_uppercase()}" }
            button { id: "theme", class: "btn btn-ghost btn-sm btn-circle", r#type: "button", aria_label: "Theme", dangerous_inner_html: "{theme_icons}" }
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// JSON for embedding inside a `<script>` element.
fn script_json(v: &impl Serialize) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "null".into())
        .replace("</", "<\\/")
        .replace("<!--", "<\\!--")
}

#[derive(Clone, PartialEq)]
enum Notice {
    None,
    Error(&'static str),
    AlreadyIn,
}

struct PageCtx<'a> {
    raffle: &'a RaffleRow,
    /// What phones see, read in one snapshot, and how they follow it.
    live: &'a service::public::Participant,
    lang: Lang,
    entrant: Option<&'a EntrantRow>,
    notice: Notice,
    form: JoinForm,
}

fn render_page(ctx: PageCtx<'_>) -> String {
    let PageCtx {
        raffle,
        live: participant,
        lang,
        entrant,
        notice,
        form,
    } = ctx;
    let t = |k: &str| lang.t(k);
    let view = &participant.view;
    let phase = view.state;
    let strings = CardStrings::from_lookup(|k| lang.t(k).to_owned());
    let now_ms = crate::model::now_unix_ms();
    let code = raffle.join_code.clone();
    let other = lang.other();
    let switch_href = format!("/j/{code}?lang={}", other.code());
    let rules_href = format!("/rules/{}?lang={}", raffle.slug, lang.code());
    let privacy_href = format!("/privacy/{}?lang={}", raffle.slug, lang.code());
    let error = match notice {
        Notice::Error(k) => Some(t(k)),
        _ => None,
    };
    let me = entrant.map(|e| e.id.clone());
    let claim_code = entrant.map(|e| e.claim_code.clone());
    let joining = entrant.is_none() && phase == Phase::Open && notice != Notice::AlreadyIn;

    // The live card's mount point: the page owns it, the card renders inside.
    let live = rsx! {
        section { id: "live", class: "empty:hidden", aria_live: "polite",
            Card {
                view: view.clone(),
                me: me.clone(),
                claim_code: claim_code.clone(),
                page_state: phase,
                now_ms,
                strings: strings.clone(),
            }
        }
    };

    let card = "card bg-base-100 shadow-sm ring-1 ring-base-300";
    let main = match entrant {
        Some(e) => rsx! {
            section { class: "ticket bg-primary text-primary-content shadow-lg animate-pop",
                div { class: "space-y-1 px-7 pt-6",
                    p { class: "text-xs font-bold uppercase tracking-[0.2em] opacity-80", "🎟️ {t(\"youre_in\")}" }
                    h2 { class: "break-words text-3xl font-extrabold", "{e.display_name}" }
                }
                div { class: "mx-7 my-4 border-t-2 border-dashed border-primary-content/40" }
                div { class: "flex flex-col items-center gap-1 px-7 pb-6 text-center",
                    span { class: "text-xs font-semibold uppercase tracking-widest opacity-75", "{t(\"claim_code\")}" }
                    strong { class: "claim-code text-5xl", "{e.claim_code}" }
                    small { class: "text-sm opacity-80", "{t(\"claim_code_hint\")}" }
                }
            }
            {live}
            button { id: "wake", class: "btn btn-outline btn-block rounded-full", r#type: "button", "{t(\"keep_screen_on\")}" }
            p { class: "text-center text-sm text-base-content/60", "{t(\"keep_open\")}" }
        },
        // The join form: no live card here.
        None if joining => {
            let name = form.name.clone();
            let email = form.email.clone();
            let join_label = format!("{} 🎟️", t("join"));
            rsx! {
                form { class: "{card}", method: "post", action: "/j/{code}?lang={lang.code()}", id: "join",
                    div { class: "card-body gap-1",
                        h2 { class: "card-title text-2xl", "{t(\"join_title\")} " span { class: "inline-block animate-float", "🎁" } }
                        if let Some(err) = error {
                            div { class: "alert alert-error alert-soft my-2", role: "alert", "{err}" }
                        }
                        fieldset { class: "fieldset",
                            legend { class: "fieldset-legend text-sm", "{t(\"display_name\")}" }
                            input { class: "input input-lg w-full", id: "name", name: "name", required: true, maxlength: "{rafi_lifecycle::entrant::MAX_NAME_CHARS}", autocomplete: "nickname", value: "{name}" }
                            p { class: "label", "{t(\"display_name_hint\")}" }
                        }
                        fieldset { class: "fieldset",
                            legend { class: "fieldset-legend text-sm", "{t(\"email\")}" }
                            input { class: "input input-lg w-full", id: "email", name: "email", r#type: "email", required: true, autocomplete: "email", inputmode: "email", value: "{email}" }
                            p { class: "label whitespace-normal", "{t(\"email_hint\")}" }
                        }
                        label { class: "mt-3 flex items-start gap-3",
                            input { class: "checkbox checkbox-primary mt-0.5", r#type: "checkbox", name: "rules", value: "on", required: true }
                            span { class: "text-sm",
                                "{t(\"accept_rules\")} "
                                a { class: "link link-primary", href: "{rules_href}", target: "_blank", "{t(\"rules\")}" }
                                " · "
                                a { class: "link link-primary", href: "{privacy_href}", target: "_blank", "{t(\"privacy\")}" }
                            }
                        }
                        label { class: "mt-2 flex items-start gap-3",
                            input { class: "checkbox mt-0.5", r#type: "checkbox", name: "marketing", value: "on" }
                            span { class: "text-sm text-base-content/80", "{t(\"marketing\")}" }
                        }
                        input { r#type: "hidden", name: "altcha", id: "altcha" }
                        // Enabled by the WebAssembly (or the fallback) once the
                        // proof-of-work is solved; it then shows `data-label`.
                        button { class: "btn btn-primary btn-lg btn-block mt-4 rounded-full", r#type: "submit", id: "submit", disabled: true, "data-label": "{join_label}",
                            span { class: "loading loading-spinner loading-md" }
                            "{t(\"solving\")}"
                        }
                    }
                }
            }
        }
        None => {
            let (emoji, msg) = match (notice.clone(), phase) {
                (Notice::AlreadyIn, _) => ("👋", t("already_in")),
                (Notice::Error(k), _) => ("⚠️", t(k)),
                (_, Phase::Ready) => ("⏳", t("reg_not_open")),
                _ => ("🔒", t("reg_closed")),
            };
            rsx! {
                section { class: "{card} items-center gap-2 p-6 text-center",
                    span { class: "text-4xl", "{emoji}" }
                    p { class: "text-lg font-semibold", role: "status", "{msg}" }
                }
                {live}
            }
        }
    };

    let title = raffle.title.clone();
    let header = header_bar(lang, &switch_href, raffle.test_mode);
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-md flex-col gap-4 px-4 pb-10 pt-4",
            {header}
            h1 { class: "text-2xl font-extrabold leading-tight", "{title}" }
            {main}
            footer { class: "mt-2 flex flex-wrap justify-center gap-x-3 gap-y-1 text-sm text-base-content/60",
                a { class: "link link-hover", href: "{rules_href}", "{t(\"rules\")}" }
                a { class: "link link-hover", href: "{privacy_href}", "{t(\"privacy\")}" }
                a { class: "link link-hover", href: "/v/{raffle.slug}", "{t(\"verify_link\")}" }
            }
        }
    });

    let config = PageConfig {
        raffle_id: raffle.id.clone(),
        entrant_id: me.clone(),
        claim_code: claim_code.clone(),
        page_state: phase,
        view: view.clone(),
        strings: strings.clone(),
        participant_url: format!("/api/r/{}/participant", raffle.slug),
        server_now_ms: now_ms,
        follow: participant.follow.clone(),
        needs_altcha: joining,
        altcha_url: PageConfig::ALTCHA_URL.into(),
        screen_on: lang.t("screen_on").to_owned(),
        keep_screen_on: lang.t("keep_screen_on").to_owned(),
    };
    let tail = format!(
        "<script type=\"application/json\" id=\"rafi-config\">{}</script><script>{LOADER}</script>",
        script_json(&config)
    );
    document(lang, &raffle.title, body, Some(tail))
}

fn not_found(lang: Lang) -> Response {
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex min-h-dvh max-w-md flex-col items-center justify-center gap-3 px-4 text-center",
            span { class: "text-6xl", "🎟️" }
            p { class: "text-xl font-bold", "{lang.t(\"not_found\")}" }
        }
    });
    (
        StatusCode::NOT_FOUND,
        Html(document(lang, "Rafi", body, None)),
    )
        .into_response()
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
    let html = render_page(PageCtx {
        raffle: &raffle,
        live: &live,
        lang,
        entrant: entrant.as_ref(),
        notice: Notice::None,
        form: JoinForm::default(),
    });
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
    let html = render_page(PageCtx {
        raffle: &raffle,
        live: &live,
        lang,
        entrant: None,
        notice,
        form,
    });
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

fn text_page(
    lang: Lang,
    title: &str,
    back: String,
    sections: Vec<(String, Vec<String>)>,
) -> Html<String> {
    let title_owned = title.to_owned();
    let back_icon = I::ArrowLeft.svg("size-4");
    let body = dioxus::ssr::render_element(rsx! {
        main { class: "mx-auto flex max-w-2xl flex-col gap-4 px-4 py-8",
            a { class: "btn btn-ghost btn-sm w-fit", href: "{back}", span { dangerous_inner_html: "{back_icon}" } "rafi" }
            h1 { class: "text-3xl font-extrabold", "{title_owned}" }
            for (heading , paras) in sections {
                section { class: "card bg-base-100 shadow-sm ring-1 ring-base-300",
                    div { class: "card-body gap-2",
                        h2 { class: "card-title text-lg", "{heading}" }
                        for p in paras {
                            p { class: "text-base-content/80", "{p}" }
                        }
                    }
                }
            }
        }
    });
    Html(document(lang, title, body, None))
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
    let prizes: Vec<String> = prizes
        .iter()
        .map(|i| {
            let sponsor = if i.sponsor.is_empty() {
                String::new()
            } else {
                format!(" ({})", i.sponsor)
            };
            format!("{} × {}{sponsor}", i.qty, i.name)
        })
        .collect();
    let organizer = if r.organizer_name.is_empty() {
        "the organizer".to_owned()
    } else {
        r.organizer_name.clone()
    };
    let sections = match lang {
        Lang::En => vec![
            ("Organizer".to_owned(), vec![format!("{organizer}{}", contact(&r))]),
            ("Prizes".to_owned(), if prizes.is_empty() { vec!["To be announced.".into()] } else { prizes }),
            ("Who can take part".to_owned(), vec![
                format!("Anyone attending the event in person who is at least {} years old. Organizers and volunteers cannot win. Sponsor employees cannot win their own sponsor's prizes.", r.min_age),
                "Entry is free. No purchase or donation is necessary. One entry per person.".into(),
            ]),
            ("How the draw works".to_owned(), vec![
                "Registration closes when the draw starts and the list of entrants is frozen and published (as a hash and as a list of anonymous IDs).".into(),
                "Each prize is drawn separately. Every draw commits to a future public randomness round from drand (League of Entropy) before that round exists, so nobody — including the organizer — can know or influence the result. Anyone can re-run every draw on the verification page.".into(),
                "Each person can win at most one prize. If more prizes remain than people, previous winners can win again.".into(),
            ]),
            ("Claiming a prize".to_owned(), vec![
                "Winners must be present and claim their prize at the event, by showing the claim code on their phone or identifying themselves to the organizer. If a winner is not present or declines, they are out of the draw and the prize is drawn again.".into(),
                "Prizes cannot be exchanged for cash.".into(),
            ]),
        ],
        Lang::Hu => vec![
            ("Szervező".to_owned(), vec![format!("{organizer}{}", contact(&r))]),
            ("Nyeremények".to_owned(), if prizes.is_empty() { vec!["Hamarosan.".into()] } else { prizes }),
            ("Ki vehet részt".to_owned(), vec![
                format!("Bárki, aki személyesen jelen van a rendezvényen és legalább {} éves. A szervezők és önkéntesek nem nyerhetnek. A szponzorok munkatársai a saját cégük nyereményeit nem nyerhetik meg.", r.min_age),
                "A részvétel ingyenes, vásárláshoz vagy adományhoz nem kötött. Személyenként egy jelentkezés.".into(),
            ]),
            ("A sorsolás menete".to_owned(), vec![
                "A sorsolás indulásakor a jelentkezés lezárul, a résztvevők listáját rögzítjük és nyilvánosságra hozzuk (hash formájában és anonim azonosítók listájaként).".into(),
                "Minden nyereményt külön sorsolunk. Minden sorsolás egy jövőbeli, nyilvános drand (League of Entropy) véletlenszám-körre hivatkozik, mielőtt az létezne, így senki — a szervező sem — tudhatja előre vagy befolyásolhatja az eredményt. Az ellenőrző oldalon bárki újrafuttathatja a sorsolásokat.".into(),
                "Mindenki legfeljebb egy nyereményt nyerhet. Ha több nyeremény marad, mint résztvevő, a korábbi nyertesek újra nyerhetnek.".into(),
            ]),
            ("A nyeremény átvétele".to_owned(), vec![
                "A nyertesnek jelen kell lennie, és a helyszínen kell átvennie a nyereményt a telefonján látható átvételi kóddal vagy a szervezőnél azonosítva magát. Ha a nyertes nincs jelen vagy visszautasítja, kiesik a sorsolásból, és a nyereményt újrasorsoljuk.".into(),
                "A nyeremény készpénzre nem váltható.".into(),
            ]),
        ],
    };
    let mut sections = sections;
    if !r.rules.trim().is_empty() {
        let heading = if lang == Lang::Hu {
            "További szabályok"
        } else {
            "Additional rules"
        };
        sections.push((
            heading.into(),
            r.rules
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect(),
        ));
    }
    let title = format!("{} — {}", lang.t("rules"), r.title);
    text_page(lang, &title, format!("/j/{}", r.join_code), sections).into_response()
}

fn contact(r: &RaffleRow) -> String {
    if r.contact_email.is_empty() {
        String::new()
    } else {
        format!(" — {}", r.contact_email)
    }
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
    let controller = if r.organizer_name.is_empty() {
        "The raffle organizer".to_owned()
    } else {
        r.organizer_name.clone()
    };
    let days_nw = app.policy.retention.nonwinners_days;
    let days_w = app.policy.retention.winners_days;
    let sections = match lang {
        Lang::En => vec![
            ("Controller".to_owned(), vec![format!("{controller}{}", contact(&r))]),
            ("What we collect and why".to_owned(), vec![
                "Your display name and email address, to run the prize draw you entered under the raffle rules (GDPR Art. 6(1)(b)): to tell entries apart, to identify winners and to deliver prizes.".into(),
                "Duplicate and abuse checks (one entry per email address, rate limiting, a proof-of-work check that runs in your browser) are based on our legitimate interest in a fair draw (Art. 6(1)(f)).".into(),
                "If you tick the optional box, we also use your email to tell you about future meetups, based on your consent (Art. 6(1)(a)). You can withdraw it at any time by writing to the contact above.".into(),
                "Providing your name and email is required to enter. Your email is never shown on the screen or shared with sponsors.".into(),
            ]),
            ("What is public".to_owned(), vec![
                "Your display name may appear on the event screen and on the verification page. The published entrant list contains only random anonymous IDs.".into(),
            ]),
            ("Cookies".to_owned(), vec![
                "We set one strictly necessary cookie that remembers your entry on this device, and one that remembers your language choice. No analytics or tracking.".into(),
            ]),
            ("Processors".to_owned(), vec![
                "Hetzner Online GmbH (Germany) hosts the application and database. Backups are stored with an EU object storage provider.".into(),
                "Restate GmbH (Germany) runs the raffle engine; it only receives anonymous IDs, never names or emails.".into(),
                "Clerk, Inc. (USA, EU–US Data Privacy Framework) authenticates the organizers only; participant data is never sent to Clerk.".into(),
                "The randomness comes from the public drand network; no personal data is sent to it.".into(),
            ]),
            ("How long we keep it".to_owned(), vec![
                format!("Personal data of entrants who did not win is erased {days_nw} days after the event. Winners' data is erased once the prize is delivered, at the latest {days_w} days after the event. Anonymous IDs and the draw log are kept so the draw stays verifiable."),
            ]),
            ("Your rights".to_owned(), vec![
                "You can request access, correction, erasure, restriction, or object to processing, and withdraw consent, by contacting the controller. You can also lodge a complaint with the Hungarian National Authority for Data Protection and Freedom of Information (NAIH, naih.hu).".into(),
            ]),
        ],
        Lang::Hu => vec![
            ("Adatkezelő".to_owned(), vec![format!("{controller}{}", contact(&r))]),
            ("Milyen adatot és miért kezelünk".to_owned(), vec![
                "A megjelenítendő nevedet és az e-mail címedet a sorsolás lebonyolításához kezeljük, amelyre a szabályok szerint jelentkeztél (GDPR 6. cikk (1) b)): a jelentkezések megkülönböztetéséhez, a nyertesek azonosításához és a nyeremények átadásához.".into(),
                "A duplikációk és visszaélések kiszűrése (egy jelentkezés e-mail címenként, kéréskorlátozás, a böngésződben futó proof-of-work ellenőrzés) a tisztességes sorsoláshoz fűződő jogos érdekünkön alapul (6. cikk (1) f)).".into(),
                "Ha bejelölöd a nem kötelező négyzetet, a hozzájárulásod alapján (6. cikk (1) a)) e-mailben értesítünk a következő meetupokról. A hozzájárulást bármikor visszavonhatod a fenti elérhetőségen.".into(),
                "A név és e-mail cím megadása a részvétel feltétele. Az e-mail címed sehol nem jelenik meg, és nem adjuk át szponzoroknak.".into(),
            ]),
            ("Mi nyilvános".to_owned(), vec![
                "A megjelenítendő neved megjelenhet a kivetítőn és az ellenőrző oldalon. A közzétett résztvevői lista csak véletlenszerű anonim azonosítókat tartalmaz.".into(),
            ]),
            ("Sütik".to_owned(), vec![
                "Egy feltétlenül szükséges sütit használunk, amely ezen az eszközön megjegyzi a jelentkezésedet, és egyet a nyelvválasztáshoz. Analitika vagy követés nincs.".into(),
            ]),
            ("Adatfeldolgozók".to_owned(), vec![
                "Hetzner Online GmbH (Németország): az alkalmazás és az adatbázis tárhelye. A biztonsági mentések EU-s objektumtárolóban vannak.".into(),
                "Restate GmbH (Németország): a sorsolómotor; csak anonim azonosítókat kap, nevet vagy e-mail címet soha.".into(),
                "Clerk, Inc. (USA, EU–USA adatvédelmi keretrendszer): kizárólag a szervezők bejelentkezése; résztvevői adat nem kerül a Clerkhez.".into(),
                "A véletlenszám a nyilvános drand hálózatból származik; oda személyes adat nem kerül.".into(),
            ]),
            ("Meddig őrizzük".to_owned(), vec![
                format!("A nem nyertes résztvevők személyes adatait a rendezvény után {days_nw} nappal töröljük. A nyertesek adatait a nyeremény átadása után, legkésőbb {days_w} nappal a rendezvény után töröljük. Az anonim azonosítókat és a sorsolási naplót megőrizzük, hogy a sorsolás ellenőrizhető maradjon."),
            ]),
            ("Jogaid".to_owned(), vec![
                "Kérhetsz hozzáférést, helyesbítést, törlést, korlátozást, tiltakozhatsz az adatkezelés ellen, és visszavonhatod a hozzájárulásodat az adatkezelőnél. Panaszt tehetsz a Nemzeti Adatvédelmi és Információszabadság Hatóságnál (NAIH, naih.hu).".into(),
            ]),
        ],
    };
    let title = format!("{} — {}", lang.t("privacy"), r.title);
    text_page(lang, &title, format!("/j/{}", r.join_code), sections).into_response()
}
