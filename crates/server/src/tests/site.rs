//! What the server serves besides the raffles: the PoW worker, `/config.js`,
//! `security.txt`, link previews, `/healthz`.

use super::*;

#[tokio::test]
async fn pow_worker_is_dioxus_altchas() {
    let app = app().await;
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    let participant = public.join("participant-dist");
    std::fs::create_dir_all(&participant).unwrap();
    // A participant directory built before the route: its copy is ignored.
    std::fs::write(participant.join("pow-worker.js"), "// stale").unwrap();
    let router = crate::build_router(
        app,
        None,
        &crate::Dirs {
            public: public.clone(),
            participant,
        },
        None,
    );
    let get = || {
        Request::get("/participant/pow-worker.js")
            .body(Body::empty())
            .unwrap()
    };

    let (status, headers, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, dioxus_altcha::WORKER_JS);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    // Revalidating: unchanged, so no body.
    for if_none_match in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
        "*".into(),
    ] {
        let mut req = get();
        req.headers_mut()
            .insert(header::IF_NONE_MATCH, if_none_match.parse().unwrap());
        let (status, headers, body) = send(&router, req).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_MODIFIED, ""),
            "{if_none_match}"
        );
        assert_eq!(headers[header::ETAG], etag.as_str());
    }
    let mut req = get();
    req.headers_mut()
        .insert(header::IF_NONE_MATCH, "\"other\"".parse().unwrap());
    let (status, _, body) = send(&router, req).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, dioxus_altcha::WORKER_JS)
    );
    std::fs::remove_dir_all(&public).ok();
}

/// `/config.js` tells the web app how the server signs organizers in: the
/// Clerk publishable key when it verifies Clerk sessions, none under dev
/// auth. The server is the one authority, so the web app can't disagree
/// with it, and one web app bundle serves every installation.
#[tokio::test]
async fn config_js_says_how_the_server_signs_in() {
    // And it applies the saved theme before first paint, which the web app's
    // shell can't do inline under its policy.
    const THEME: &str = crate::pages::APPLY_THEME;
    // Which Rafi, for the footers: the version, its source and its verifier.
    let build = format!(
        "\"version\":{:?},\"sourceUrl\":{:?},\"verifierUrl\":{:?},\"guideUrl\":{:?}",
        crate::build_info::VERSION,
        crate::build_info::source_url(),
        crate::build_info::verifier_url(),
        crate::build_info::guide_url(),
    );
    let get = || Request::get("/config.js").body(Body::empty()).unwrap();

    let router = routes(app().await);
    let (status, headers, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    // A rotated key or a switched mode takes effect on the next load.
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        body,
        format!(
            "window.__RAFI_CONFIG__ = {{\"clerkPublishableKey\":{:?},{build}}};\n{THEME}\n",
            Config::for_tests().clerk_publishable_key.unwrap()
        )
    );

    // Dev auth: no key, whatever is configured, so the admin shows its
    // dev-auth badge instead of a sign-in nobody checks.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let router = routes(app_with(cfg).await);
    let (status, _, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        format!("window.__RAFI_CONFIG__ = {{\"clerkPublishableKey\":null,{build}}};\n{THEME}\n")
    );
}

/// `/.well-known/security.txt` names the operator, Rafi's policy and an
/// expiry under a year away (RFC 9116).
#[tokio::test]
async fn security_txt_names_the_operator() {
    let app = app().await;
    let operator = app.cfg.operator.email.clone();
    let router = routes(app);
    let (status, headers, body) = send(
        &router,
        Request::get("/.well-known/security.txt")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/plain; charset=utf-8");
    assert!(!operator.is_empty());
    for line in [
        format!("Contact: mailto:{operator}"),
        format!("Policy: {}/security/policy", crate::build_info::SOURCE_URL),
        "Preferred-Languages: en".to_owned(),
        "Canonical: http://localhost:8080/.well-known/security.txt".to_owned(),
    ] {
        assert!(body.lines().any(|l| l == line), "{line}: {body}");
    }
    let expires = body
        .lines()
        .find_map(|l| l.strip_prefix("Expires: "))
        .and_then(|e| rafi_lifecycle::dates::Date::parse(&e[..10]))
        .unwrap();
    let today = rafi_lifecycle::dates::Date::of_utc(rafi_store::now_ms() as i64 / 1000);
    let days = expires.days() - today.days();
    assert!((1..365).contains(&days), "{days} days: {body}");
}

/// A join link pasted into a chat previews as the raffle: its title, a line,
/// the image (absolute), and the icons every page links.
#[tokio::test]
async fn a_join_link_previews_as_the_raffle() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let (status, _, page) = send(&router, get(&format!("/j/{}", raffle.join_code))).await;
    assert_eq!(status, StatusCode::OK);
    let title = raffles::get(&app.db, &raffle.id)
        .await
        .unwrap()
        .unwrap()
        .title;
    for expected in [
        format!("<meta property=\"og:title\" content=\"{title}\">"),
        "<meta property=\"og:image\" content=\"http://localhost:8080/assets/og.png\">".to_owned(),
        "<meta name=\"twitter:card\" content=\"summary_large_image\">".to_owned(),
        "<link rel=\"icon\" href=\"/assets/favicon.svg\" type=\"image/svg+xml\">".to_owned(),
    ] {
        assert!(page.contains(&expected), "{expected}: {page}");
    }
}

/// `/healthz` says the server is up and which Rafi it is; the version is the
/// build's, as the web app's footers and `whoami` have it.
#[tokio::test]
async fn healthz_names_the_version() {
    let router = routes(app().await);
    let (status, _, body) = send(
        &router,
        Request::get("/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, format!("ok {}", crate::build_info::VERSION))
    );
}
