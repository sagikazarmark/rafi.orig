//! The public JSON and pages: the full and participant views, drafts, the
//! verification data.

use super::*;

#[tokio::test]
async fn public_view_has_no_personal_data() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    join(
        &router,
        &raffle.join_code,
        "Visible",
        "secret-address@example.com",
    )
    .await;
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let state = full(&app, &raffle.id).await.drawn.unwrap();
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        DrawRequest {
            item: state.items()[0].id.clone(),
        },
        "c",
    )
    .await
    .unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.items()[0].status() == ItemStatus::Drawn)
    })
    .await;

    let (status, _, body) = send(
        &router,
        Request::get(format!("/api/r/{}/view", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claim = entrants::list(&app.db, &raffle.id).await.unwrap()[0]
        .claim_code
        .clone()
        .unwrap();
    assert!(body.contains("Visible"));
    assert!(!body.contains("secret-address"), "email leaked: {body}");
    assert!(!body.contains(&format!("\"{claim}\"")), "claim code leaked");
    let (_, headers, verify) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(!verify.contains("secret-address"));

    // verify.json names its format and version; the version is its ETag, and
    // a reader revalidates rather than keeps a copy (a name erased later
    // must never come from a cache).
    let data: rafi_api::VerifyData = serde_json::from_str(&verify).unwrap();
    assert_eq!(data.format, rafi_api::VERIFY_FORMAT);
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert_eq!(etag, format!("\"v{}\"", data.v));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let (status, headers, body) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_MODIFIED, ""));
    assert_eq!(headers[header::ETAG], etag);
    // Erasing a name moves the version, so the ETag changes with the content.
    let hidden = &entrants::list(&app.db, &raffle.id).await.unwrap()[0];
    service::remove_entrant(&app, ACTOR, &raffle.id, &hidden.id, false)
        .await
        .unwrap_err();
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &hidden.id,
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, headers, body) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(headers[header::ETAG], etag);
    assert!(
        !body.contains("Visible"),
        "the hidden name is masked: {body}"
    );
}

async fn version(app: &AppState, id: &str) -> u64 {
    raffles::get(&app.db, id).await.unwrap().unwrap().version as u64
}

/// The full view's `v` is the raffle's `version` column, which every write
/// that changes a screen bumps; the view still carries the server's clock.
#[tokio::test]
async fn full_view_v_is_the_version_column() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let url = format!("/api/r/{}/view", raffle.slug);

    let (status, headers, view) = get_json(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let v0 = view["v"].as_u64().unwrap();
    assert_eq!(v0, version(&app, &raffle.id).await);
    assert!(view["server_now_ms"].as_u64().unwrap() > 0);
    assert_eq!(
        view["follow"],
        serde_json::json!({"kind": "poll", "every_ms": 2500}),
        "no push natively: clients poll"
    );

    // Polling again without a write: the same version.
    let (_, _, again) = get_json(&router, &url).await;
    assert_eq!(again["v"].as_u64().unwrap(), v0);

    add_one(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let (_, _, view) = get_json(&router, &url).await;
    let v1 = view["v"].as_u64().unwrap();
    assert!(v1 > v0, "{v1} > {v0}");
    assert_eq!(v1, version(&app, &raffle.id).await);
    assert_eq!(view["entrant_count"], 1);

    // The admin's copy is the same view.
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.view.v, v1);
}

/// What phones poll: the participant view, with no personal data beyond the
/// names of drawn entrants (masked when hidden), never cached.
#[tokio::test]
async fn participant_view_has_no_personal_data() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1), ("Other", 1)]).await;
    for (name, email) in [("Anna", "anna@example.com"), ("Bence", "bence@example.com")] {
        join(&router, &raffle.join_code, name, email).await;
    }
    let url = format!("/api/r/{}/participant", raffle.slug);
    let (status, headers, view) = get_json(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(view["state"], "open");
    assert_eq!(view["prizes_left"], 2);
    assert_eq!(view["v"].as_u64().unwrap(), version(&app, &raffle.id).await);
    let text = view.to_string();
    for secret in ["Anna", "Bence", "@example.com"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }

    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let item = r.drawn.as_ref().unwrap().items()[0].id.clone();
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        DrawRequest { item: item.clone() },
        "c",
    )
    .await
    .unwrap();
    let r = wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&item).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    let winner_id = r
        .drawn
        .as_ref()
        .unwrap()
        .item(&item)
        .unwrap()
        .winner()
        .cloned()
        .unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    let winner = entrants
        .iter()
        .find(|e| e.id == winner_id.as_str())
        .unwrap();
    let other = entrants
        .iter()
        .find(|e| e.id != winner_id.as_str())
        .unwrap();

    let (_, _, view) = get_json(&router, &url).await;
    let text = view.to_string();
    assert_eq!(view["last"]["display_name"], winner.display_name.as_str());
    assert_eq!(view["winners"][0]["entrant_id"], winner_id.as_str());
    assert!(
        !text.contains(&other.display_name),
        "a non-winner's name: {text}"
    );
    assert!(!text.contains("@example.com"), "an email: {text}");
    for e in &entrants {
        assert!(
            !text.contains(&format!("\"{}\"", e.claim_code.as_deref().unwrap())),
            "a claim code: {text}"
        );
    }

    // A hidden winner's name is masked.
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        winner_id.as_str(),
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, _, view) = get_json(&router, &url).await;
    assert!(!view.to_string().contains(&winner.display_name));

    // The participant page embeds the same view and renders its card.
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("id=\"rafi-config\""));
    assert!(page.contains(&format!("\"participant_url\":\"{url}\"")));
    assert!(page.contains(r#""follow":{"kind":"poll","every_ms":2500}"#));
    assert!(
        !page.contains(&other.display_name),
        "a non-winner's name on the page"
    );
}

#[tokio::test]
async fn drafts_are_private() {
    let app = app().await;
    let router = routes(app.clone());
    let id = create(&app, OWNER, "Secret").await;
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let (status, _, _) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for path in ["view", "participant"] {
        let (status, _, _) = send(
            &router,
            Request::get(format!("/api/r/{}/{path}", raffle.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    // Other organizers cannot see it either.
    assert!(matches!(
        service::admin(&app, "someone_else", &id).await,
        Err(service::Error::NotFound)
    ));
}

/// The public page only trusts public drand chains: a mock raffle never passes there.
#[tokio::test]
async fn mock_raffles_are_not_publicly_verified() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    add_one(&app, ACTOR, &raffle.id, "Person", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let public = rafi_raffle::verify_public(&data.manifest, &data.manifest_hash, &data.events);
    assert!(!public.beacons_verified && !public.ok());
}
