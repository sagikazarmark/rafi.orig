//! Security headers and the Content-Security-Policy of every page, framing and
//! indexing.

use super::*;

/// The Content-Security-Policy tests' router, as the Worker composes it:
/// the server's routes, and a shell at every other path.
fn with_shell(app: &AppState) -> Router {
    routes(app.clone()).fallback_service(crate::shell_routes(
        app,
        Some(crate::Shell::new(TEST_SHELL).unwrap()),
    ))
}

/// A stand-in for the web app's `index.html`: no inline script.
const TEST_SHELL: &str = "<!doctype html><html><head><script src=\"/config.js\"></script></head><body><div id=\"main\"></div></body></html>";

/// `name`'s value in `headers`, empty without one.
fn header_str(headers: &header::HeaderMap, name: header::HeaderName) -> String {
    headers
        .get(name)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default()
}

/// The enforced Content-Security-Policy in `headers`.
fn policy(headers: &header::HeaderMap) -> String {
    header_str(headers, header::CONTENT_SECURITY_POLICY)
}

/// The sources of `policy`'s `name` directive.
fn directive(policy: &str, name: &str) -> Vec<String> {
    policy
        .split("; ")
        .find_map(|d| d.strip_prefix(name)?.strip_prefix(' '))
        .map(|sources| sources.split(' ').map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The server's pages allow what they carry inline by its hash, and nothing
/// else inline: no eval, no inline styles, no Clerk; they are never framed,
/// and they connect to the live channel's socket by name.
#[tokio::test]
async fn server_pages_allow_their_inline_scripts_by_hash() {
    let app = app().await;
    let router = with_shell(&app);
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let join = format!("/j/{}", raffle.join_code);
    for path in [
        join.clone(),
        format!("/rules/{}", raffle.slug),
        format!("/privacy/{}", raffle.slug),
        "/j/no-such-code".to_owned(),
    ] {
        let (_, headers, page) = send(&router, get(&path)).await;
        let p = policy(&headers);
        let scripts = directive(&p, "script-src");
        let inline = crate::csp::inline_scripts(&page);
        assert!(!inline.is_empty(), "{path}: the head script at least");
        for script in inline {
            assert!(
                scripts.contains(&crate::csp::hash_source(script)),
                "{path}: {script:?} isn't allowed by {p}"
            );
        }
        for loose in ["'unsafe-eval'", "'unsafe-inline'"] {
            assert!(!p.contains(loose), "{path}: {p}");
        }
        assert_eq!(directive(&p, "style-src"), ["'self'"], "{path}");
        assert_eq!(directive(&p, "img-src"), ["'self'", "data:"], "{path}");
        assert_eq!(
            directive(&p, "connect-src"),
            ["'self'", "ws://localhost:8080"],
            "{path}"
        );
        assert_eq!(directive(&p, "form-action"), ["'self'"], "{path}");
        assert_eq!(directive(&p, "frame-ancestors"), ["'none'"], "{path}");
        assert!(!p.contains("clerk"), "{path}: {p}");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
    }
    // The join page runs the participant build's loader, inline too.
    let (_, _, page) = send(&router, get(&join)).await;
    assert_eq!(crate::csp::inline_scripts(&page).len(), 2);
}

/// The installation's documents: the server pages' policy, and images from
/// any https: origin, which the operator's Markdown may show.
#[tokio::test]
async fn the_documents_also_show_images_from_elsewhere() {
    let app = app().await;
    let router = with_shell(&app);
    let (_, headers, _) = send(&router, get("/j/no-such-code")).await;
    let server_page = policy(&headers);
    for path in ["/privacy", "/terms"] {
        let (status, headers, _) = send(&router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let p = policy(&headers);
        assert_eq!(
            directive(&p, "img-src"),
            ["'self'", "data:", "https:"],
            "{path}"
        );
        assert_eq!(
            directive(&p, "script-src"),
            directive(&server_page, "script-src"),
            "{path}"
        );
    }
}

/// The web app's shell, at every path of its own: Dioxus's eval, inline
/// styles and everything Clerk needs for the publishable key; nothing inline.
#[tokio::test]
async fn the_web_app_allows_eval_inline_styles_and_clerk() {
    let app = app().await;
    let router = with_shell(&app);
    let clerk = dioxus_clerk::csp::ClerkCsp::from_publishable_key(
        crate::config::TEST_CLERK_PUBLISHABLE_KEY,
    )
    .unwrap();
    for path in ["/", "/admin", "/admin/r/abc", "/index.html"] {
        let (status, headers, body) = send(&router, get(path)).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::OK, TEST_SHELL),
            "{path}"
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{path}");
        let p = policy(&headers);
        let scripts = directive(&p, "script-src");
        for source in ["'self'", "'wasm-unsafe-eval'", "'unsafe-eval'"] {
            assert!(
                scripts.contains(&source.to_owned()),
                "{path}: {source} in {p}"
            );
        }
        for (name, sources) in clerk.directives() {
            let allowed = directive(&p, name);
            for source in sources {
                assert!(
                    allowed.contains(source),
                    "{path}: Clerk's {name} {source} in {p}"
                );
            }
        }
        assert!(!p.contains("'sha256-"), "{path}: nothing inline: {p}");
        assert!(directive(&p, "style-src").contains(&"'unsafe-inline'".to_owned()));
        assert_eq!(directive(&p, "frame-ancestors"), ["'none'"], "{path}");
    }
}

/// A revalidated shell keeps its policy: a browser keeps a 304's headers in
/// place of the cached response's, so a 304 with less would strip it.
#[tokio::test]
async fn the_web_app_keeps_its_policy_when_revalidated() {
    let app = app().await;
    let router = with_shell(&app);
    for path in ["/admin", "/s/some-slug"] {
        let (_, fresh, _) = send(&router, get(path)).await;
        let mut req = get(path);
        req.headers_mut()
            .insert(header::IF_NONE_MATCH, fresh[header::ETAG].clone());
        let (status, revalidated, body) = send(&router, req).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_MODIFIED, ""),
            "{path}"
        );
        for name in [header::CONTENT_SECURITY_POLICY, header::X_FRAME_OPTIONS] {
            assert_eq!(revalidated.get(&name), fresh.get(&name), "{path}: {name}");
        }
    }
}

/// The projector and the verification page may be framed (venues and
/// meetup sites show them); nothing else of the web app's may.
#[tokio::test]
async fn only_the_projector_and_the_verification_page_may_be_framed() {
    let app = app().await;
    let router = with_shell(&app);
    for path in ["/s/some-slug", "/v/some-slug"] {
        let (_, headers, _) = send(&router, get(path)).await;
        let p = policy(&headers);
        assert!(p.contains("'unsafe-eval'"), "{path}: the web app's: {p}");
        assert!(!p.contains("frame-ancestors"), "{path}: {p}");
        assert!(!headers.contains_key(header::X_FRAME_OPTIONS), "{path}");
    }
}

/// The public pages that list entrants' names, the projector and the
/// verification page, and the JSON with the names behind them, ask search
/// engines not to index them, revalidated too.
#[tokio::test]
async fn pages_with_names_are_not_indexed() {
    let app = app().await;
    let router = with_shell(&app);
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    add_one(&app, ACTOR, &raffle.id, "Anna Kovács", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let robots = |headers: &header::HeaderMap| {
        header_str(headers, header::HeaderName::from_static("x-robots-tag"))
    };

    for path in [
        format!("/s/{}", raffle.slug),
        format!("/v/{}", raffle.slug),
        format!("/api/r/{}/view", raffle.slug),
        format!("/api/r/{}/verify.json", raffle.slug),
    ] {
        let (status, headers, body) = send(&router, get(&path)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(robots(&headers), "noindex", "{path}");
        if !path.ends_with("/view") {
            let mut req = get(&path);
            req.headers_mut()
                .insert(header::IF_NONE_MATCH, headers[header::ETAG].clone());
            let (status, headers, _) = send(&router, req).await;
            assert_eq!(status, StatusCode::NOT_MODIFIED, "{path}");
            assert_eq!(robots(&headers), "noindex", "{path}: revalidated");
        }
    }
    // By path: not the rest of the web app.
    for path in ["/", "/admin"] {
        let (_, headers, _) = send(&router, get(path)).await;
        assert_eq!(robots(&headers), "", "{path}");
    }
}

/// What isn't a page only denies framing; a miss among the server's files
/// and APIs is a 404, not the shell; the shell is for reading.
#[tokio::test]
async fn what_isnt_a_page_only_denies_framing() {
    let app = app().await;
    let router = with_shell(&app);
    let (_, headers, _) = send(&router, get("/healthz")).await;
    assert_eq!(policy(&headers), crate::csp::NO_FRAMING);
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    for path in [
        "/assets/rafi-web-dxh0.js",
        "/participant/x.js",
        "/api/nothing",
        "/legal/terms.md",
    ] {
        let (status, headers, body) = send(&router, get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(policy(&headers), crate::csp::NO_FRAMING, "{path}");
    }
    let (status, headers, _) = send(
        &router,
        Request::post("/admin").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(headers.contains_key(header::ALLOW));
}

/// Dev auth loads no Clerk, so the web app's policy names none of it.
#[tokio::test]
async fn under_dev_auth_the_web_app_allows_nothing_of_clerks() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let (_, headers, _) = send(&with_shell(&app), get("/admin")).await;
    let p = policy(&headers);
    assert!(!p.contains("clerk") && !p.contains("cloudflare"), "{p}");
    assert_eq!(directive(&p, "worker-src"), ["'self'"]);
    assert_eq!(directive(&p, "frame-src"), Vec::<String>::new());
}

/// `RAFI_CSP=report-only`: every page's policy is reported, not enforced;
/// framing stays denied where it was.
#[tokio::test]
async fn report_only_reports_the_policies_and_still_denies_framing() {
    let mut cfg = Config::for_tests();
    cfg.csp = crate::config::CspMode::ReportOnly;
    let app = app_with(cfg).await;
    let router = with_shell(&app);
    for path in ["/admin", "/terms", "/j/no-such-code"] {
        let (_, headers, _) = send(&router, get(path)).await;
        assert_eq!(policy(&headers), crate::csp::NO_FRAMING, "{path}");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
        let reported = header_str(&headers, header::CONTENT_SECURITY_POLICY_REPORT_ONLY);
        assert!(
            reported.starts_with("default-src 'self'"),
            "{path}: {reported}"
        );
    }
    let (_, headers, _) = send(&router, get("/s/some-slug")).await;
    assert!(!headers.contains_key(header::CONTENT_SECURITY_POLICY));
    assert!(!headers.contains_key(header::X_FRAME_OPTIONS));
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY_REPORT_ONLY));
}
