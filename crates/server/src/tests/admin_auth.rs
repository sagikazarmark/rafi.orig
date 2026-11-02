//! The admin API's authentication and authorization: Clerk sessions, dev auth,
//! organization tiers.

use super::*;

#[tokio::test]
async fn admin_requires_clerk_session() {
    use dioxus_clerk::testing::TestClerk;
    let app = app().await;
    let clerk = TestClerk::new().unwrap();
    // A stand-in for the web bundle.
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&public).unwrap();
    std::fs::write(
        public.join("index.html"),
        "<!doctype html><html><head></head><body><div id=\"main\"></div></body></html>",
    )
    .unwrap();
    // Its files are under assets/.
    std::fs::create_dir_all(public.join("assets")).unwrap();
    std::fs::write(public.join("assets/rafi-web.js"), "// web").unwrap();
    // And for the participant build: a hashed asset and the fallback, as
    // the build leaves them, a precompressed variant next to the asset,
    // and the loader this server compiled in.
    let participant = public.join("participant-dist");
    std::fs::create_dir_all(participant.join("assets")).unwrap();
    let glue = "assets/rafi-participant-dxh0123456789abcdef.js";
    std::fs::write(participant.join(glue), "// glue").unwrap();
    std::fs::write(participant.join(format!("{glue}.gz")), "gzipped bytes").unwrap();
    std::fs::write(participant.join("fallback.js"), "// fallback").unwrap();
    std::fs::write(participant.join("index.html"), crate::pages::LOADER).unwrap();
    let dirs = crate::Dirs {
        public: public.clone(),
        participant,
    };
    // The directory holds the build this server was compiled against;
    // another build's loader (other names) is refused.
    dirs.check_participant().unwrap();
    std::fs::write(
        dirs.participant.join("index.html"),
        "<script>other</script>",
    )
    .unwrap();
    let err = dirs.check_participant().unwrap_err().to_string();
    assert!(err.contains("not the participant build"), "{err}");
    // Clerk's layer as both hosts build it: the public origin is the one
    // authorized party.
    let router = crate::build_router(
        app.clone(),
        Some(crate::auth::clerk_layer(&app.cfg, clerk.config().unwrap()).unwrap()),
        &dirs,
        dirs.shell().unwrap(),
    );
    let get = |path: &str| Request::get(path).body(Body::empty()).unwrap();
    let asset = format!("/participant/{glue}");
    let (status, headers, body) = send(&router, get(&asset)).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// glue"));
    // Named by its hash, so a phone keeps it for good; still with an ETag,
    // and a 304 for a phone that revalidates anyway.
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let etag = headers[header::ETAG].clone();
    let mut req = get(&asset);
    req.headers_mut().insert(header::IF_NONE_MATCH, etag);
    let (status, headers, body) = send(&router, req).await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_MODIFIED, ""));
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let mut req = get(&asset);
    req.headers_mut()
        .insert(header::ACCEPT_ENCODING, "gzip, br".parse().unwrap());
    let (status, headers, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(body, "gzipped bytes", "the precompressed file is served");
    // Another build's name (a page rendered before a deploy) is a plain 404:
    // nothing a browser would keep.
    let (status, headers, _) = send(
        &router,
        get("/participant/assets/rafi-participant-dxhfedcba9876543210.js"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(headers.get(header::CACHE_CONTROL).is_none(), "{headers:?}");
    // The fallback keeps its name, so phones revalidate it.
    let (status, headers, body) = send(&router, get("/participant/fallback.js")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// fallback"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(headers.contains_key(header::ETAG));

    let (status, headers, body) = send(&router, admin_call("whoami", "{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("Please sign in."), "{body}");
    // Behind Clerk's layer, every response has an ID, refusals included.
    let id = headers[crate::trace::REQUEST_ID].to_str().unwrap();
    assert_eq!(id.len(), 16, "{id}");

    let mut req = admin_call("whoami", "{}");
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert!(status.is_success(), "{status}: {body}");
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.owner_id, "user_2abc");

    // A token minted for this origin passes; one minted for another site on
    // the same Clerk instance doesn't (`azp`).
    let for_origin = |party: &str| {
        let mut req = admin_call("whoami", "{}");
        req.headers_mut().insert(
            header::COOKIE,
            clerk
                .cookie_for(&clerk.session("user_2abc").with_authorized_party(party))
                .unwrap()
                .parse()
                .unwrap(),
        );
        req
    };
    let (status, _, body) = send(&router, for_origin(&app.cfg.public_url)).await;
    assert!(status.is_success(), "our origin: {status}: {body}");
    let (status, _, body) = send(&router, for_origin("https://other.example")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "another origin: {body}");

    // The arguments must be JSON: what a cross-site form could send is refused.
    let mut req = Request::post("/api/admin/create_raffle")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(
            r#"{"title":"x","event_date":"2026-10-28","event_ends_at":null}"#,
        ))
        .unwrap();
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
    assert!(
        service::list(&app, "user_2abc").await.unwrap().is_empty(),
        "nothing was created"
    );

    // Even a function without arguments: an empty POST is what any page
    // could send, and this one erases the caller's data.
    let mut req = admin_call(
        "create_raffle",
        r#"{"title":"Kept","event_date":"2026-10-28","event_ends_at":null}"#,
    );
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert!(status.is_success(), "{status}: {body}");
    let empty_post = |headers: &[(&'static str, &'static str)]| {
        let mut req = Request::post("/api/admin/remove_my_data")
            .body(Body::empty())
            .unwrap();
        for (name, value) in headers {
            req.headers_mut().insert(*name, value.parse().unwrap());
        }
        req.headers_mut().insert(
            header::COOKIE,
            clerk.cookie("user_2abc").unwrap().parse().unwrap(),
        );
        req
    };
    let (status, _, body) = send(&router, empty_post(&[])).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
    // JSON from a sibling subdomain (the same site, so the Lax cookie came
    // along) is refused by what the browser says of its origin.
    let (status, _, body) = send(
        &router,
        empty_post(&[
            ("content-type", "application/json"),
            ("sec-fetch-site", "same-site"),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains(rafi_api::admin::codes::CROSS_SITE), "{body}");
    assert_eq!(
        service::list(&app, "user_2abc").await.unwrap().len(),
        1,
        "nothing was removed"
    );
    // From the page itself it goes through.
    let (status, _, body) = send(
        &router,
        empty_post(&[
            ("content-type", "application/json"),
            ("sec-fetch-site", "same-origin"),
        ]),
    )
    .await;
    assert!(status.is_success(), "{status}: {body}");
    assert!(service::list(&app, "user_2abc").await.unwrap().is_empty());

    // The web app, with a single-page fallback for its own routes, and the
    // security headers: the admin can't be framed, the projector and the
    // verification page can.
    for path in [
        "/",
        "/admin",
        "/admin/r/abc",
        "/s/some-slug",
        "/v/some-slug",
    ] {
        let (status, headers, body) =
            send(&router, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("<div id=\"main\">"), "{path}: {body}");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
        assert_eq!(
            headers[header::REFERRER_POLICY],
            "strict-origin-when-cross-origin",
            "{path}"
        );
        let framed = path.starts_with("/s/") || path.starts_with("/v/");
        // The web app's policy (`csp_by_page` has the rest), framing denied
        // but where it may be framed.
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.starts_with("default-src 'self'"), "{path}: {policy}");
        assert_eq!(
            policy.contains("frame-ancestors 'none'"),
            !framed,
            "{path}: {policy}"
        );
        assert_eq!(
            headers.contains_key(header::X_FRAME_OPTIONS),
            !framed,
            "{path}"
        );
        assert!(headers.contains_key(crate::trace::REQUEST_ID), "{path}");
    }
    // The web app's files, natively from its directory, with the headers
    // too; a miss there is a 404, not the shell.
    let (status, headers, body) = send(&router, get("/assets/rafi-web.js")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// web"));
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (status, _, _) = send(&router, get("/assets/missing.js")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The shell at its own path too, under its policy.
    let (status, headers, body) = send(&router, get("/index.html")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<div id=\"main\">"), "{body}");
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("'unsafe-eval'")
    );
    // The participant build's files carry the headers too.
    let (_, headers, _) = send(&router, get("/participant/fallback.js")).await;
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    let (_, headers, _) = send(&router, get(&asset)).await;
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    // And on what the server renders itself.
    let (_, headers, _) = send(
        &router,
        Request::get("/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    std::fs::remove_dir_all(&public).ok();
}

#[tokio::test]
async fn admin_api_needs_a_session_unless_dev_auth() {
    // No session and no dev auth: every admin function is refused.
    let router = routes(app().await);
    for name in [
        "whoami",
        "list_raffles",
        "create_raffle",
        "no_such_function",
    ] {
        let (status, _, body) = send(&router, admin_call(name, r#"{"title":"x"}"#)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}: {body}");
        let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (rafi_api::admin::codes::UNAUTHORIZED, "Please sign in.")
        );
    }

    // Dev auth: the organizer is `dev`.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    let router = routes(app.clone());

    let (status, _, body) = send(&router, admin_call("whoami", "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.owner_id, "dev");

    let create = r#"{"title":"Meetup #1","event_date":"2026-10-28","event_ends_at":null}"#;
    let (status, _, body) = send(&router, admin_call("create_raffle", create)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id: String = serde_json::from_str(&body).unwrap();

    let (status, _, body) = send(&router, admin_call("list_raffles", "{}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: Vec<rafi_api::RaffleSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Meetup #1");

    // Unit results are JSON `null`.
    let (status, _, body) = send(
        &router,
        admin_call(
            "set_lifecycle",
            &format!(r#"{{"id":"{id}","action":"publish"}}"#),
        ),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "null"));
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Ready
    );

    // Errors carry the service's message and a matching status.
    let (status, _, body) = send(
        &router,
        admin_call(
            "create_raffle",
            r#"{"title":" ","event_date":"2026-10-28","event_ends_at":null}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (err.code.as_str(), err.message.as_str()),
        ("title", "The title must be 1–120 characters."),
        "a refusal's code is its kind"
    );
    let (status, _, body) = send(
        &router,
        admin_call(
            "create_raffle",
            r#"{"title":"Meetup","event_date":"soon","event_ends_at":null}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("calendar day"), "{body}");
    let code = |body: &str| {
        serde_json::from_str::<rafi_api::admin::ApiError>(body)
            .unwrap()
            .code
    };
    let (status, headers, body) = send(&router, admin_call("get_raffle", r#"{"id":"nope"}"#)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(code(&body), rafi_api::admin::codes::NOT_FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "errors too");
    let (status, _, body) = send(&router, admin_call("get_raffle", "{}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), rafi_api::admin::codes::INVALID_ARGUMENTS);
    let (status, _, body) = send(&router, admin_call("no_such_function", "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(code(&body), rafi_api::admin::codes::NOT_FOUND);
}

/// In an organization, members run raffles, managers (the custom role or
/// permission) also create and configure them, and admins also accept the
/// terms and remove the organization's data. A personal account does
/// everything.
#[tokio::test]
async fn organization_tiers_gate_the_admin_api() {
    use dioxus_clerk::testing::TestClerk;
    let app = app().await;
    let clerk = TestClerk::new().unwrap();
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(public.join("participant-dist")).unwrap();
    std::fs::write(public.join("index.html"), "<!doctype html>").unwrap();
    let router = crate::build_router(
        app.clone(),
        Some(clerk.layer().unwrap()),
        &crate::Dirs {
            public: public.clone(),
            participant: public.join("participant-dist"),
        },
        None,
    );
    let call = async |cookie: &str, name: &str, args: &str| -> (StatusCode, String) {
        let mut req = admin_call(name, args);
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (status, _, body) = send(&router, req).await;
        (status, body)
    };
    let org = |user: &str, role: &str| {
        clerk
            .cookie_for(
                &clerk
                    .session(user)
                    .with_organization("org_cnb")
                    .with_organization_role(role),
            )
            .unwrap()
    };
    let member = org("user_member", "org:member");
    let manager = org("user_manager", "org:manager");
    let admin = org("user_admin", "org:admin");
    let by_permission = clerk
        .cookie_for(
            &clerk
                .session("user_perm")
                .with_organization("org_cnb")
                .with_organization_role("org:member")
                .with_organization_permissions(["org:raffles:manage"]),
        )
        .unwrap();
    let create = r#"{"title":"Org raffle","event_date":"2026-10-28","event_ends_at":null}"#;

    // Who they are.
    let (_, body) = call(&member, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (me.owner_id.as_str(), me.access, me.shared),
        ("org_cnb", rafi_api::Access::Member, true)
    );
    let (_, body) = call(&by_permission, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(
        me.access,
        rafi_api::Access::Manager,
        "the permission makes a manager"
    );
    let (_, body) = call(&admin, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.access, me.shared), (rafi_api::Access::Admin, true));

    // Members run raffles; managers also create them.
    let (status, body) = call(&member, "create_raffle", create).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("admin or manager"), "{body}");
    let (status, body) = call(&manager, "create_raffle", create).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id: String = serde_json::from_str(&body).unwrap();
    let (status, body) = call(&member, "save_prize", &format!(r#"{{"id":"{id}","prize":{{"id":null,"name":"Book","sponsor":"","description":"","qty":1,"position":0}}}}"#)).await;
    assert_eq!(status, StatusCode::OK, "a member sets up prizes: {body}");
    let (status, body) = call(
        &member,
        "set_lifecycle",
        &format!(r#"{{"id":"{id}","action":"publish"}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a member runs the raffle: {body}");
    // The audit log tells the members apart: the owner is the organization,
    // the user is whoever acted.
    let who: Vec<(String, String, String)> = app
        .db
        .query("SELECT actor, user_id, action FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        who,
        [
            ("org_cnb".into(), "user_manager".into(), "create".into()),
            ("org_cnb".into(), "user_member".into(), "ready".into()),
        ]
    );
    let (status, body) = call(&member, "delete_raffle", &format!(r#"{{"id":"{id}"}}"#)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = call(&member, "remove_my_data", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = call(&manager, "remove_my_data", "").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a manager isn't an admin: {body}"
    );
    assert!(
        body.contains("Only an organization admin can do this."),
        "{body}"
    );
    assert!(
        raffles::get(&app.db, &id).await.unwrap().is_some(),
        "nothing happened"
    );

    // A personal account is its own admin.
    let person = clerk.cookie("user_solo").unwrap();
    let (_, body) = call(&person, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.access, me.shared), (rafi_api::Access::Admin, false));
    std::fs::remove_dir_all(&public).ok();
}
