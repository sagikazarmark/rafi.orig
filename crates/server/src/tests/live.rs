//! The live channel: ending it, nudging it, destroying it.

use super::*;

#[tokio::test]
async fn ending_live_updates() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    assert!(
        !live.notified().is_empty(),
        "lifecycle changes nudge the channel"
    );
    let err = service::end_live(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::Error::Refused(_)), "{err}");
    assert!(live.ended().is_empty());

    let raffle = finished_raffle(&app).await;
    assert!(raffle.live_ended_at.is_none());
    let view = crate::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.follow, Follow::Partyline { base_url: None });
    assert!(!view.live_ended);

    service::end_live(&app, ACTOR, &raffle.id).await.unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    let ended = r.live_ended_at.expect("live_ended_at is set");
    assert_eq!(live.ended(), [RaffleId::new(&raffle.id)]);
    let view = crate::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.follow, Follow::poll(), "ended: plain polling");
    assert!(view.live_ended);

    // Again: the channel is destroyed again, the time stays.
    service::end_live(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(live.ended().len(), 2);
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.live_ended_at, Some(ended));
    // Other organizers can't.
    assert!(matches!(
        service::end_live(&app, service::Actor::solo("someone_else"), &raffle.id).await,
        Err(service::Error::NotFound)
    ));
}

/// Deleting and purging a raffle destroy its channel; admin edits that phones
/// see nudge it, joins don't.
#[tokio::test]
async fn writes_nudge_and_destroy_the_channel() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let nudges = || live.notified().len();

    let before = nudges();
    join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    add_one(&app, ACTOR, &raffle.id, "Bence", None)
        .await
        .unwrap();
    assert_eq!(nudges(), before, "joins don't nudge");

    // Entrants who joined in the same second are listed by ID.
    let anna = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Anna")
        .unwrap()
        .id;
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &anna,
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(nudges(), before + 1, "hiding a name nudges");
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(nudges(), before + 2, "the freeze nudges");

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(live.ended(), [RaffleId::new(&raffle.id)]);
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Purged);
    assert!(r.live_ended_at.is_some(), "a purge ends live updates");

    let other = create(&app, OWNER, "Other").await;
    service::delete(&app, ACTOR, &other).await.unwrap();
    assert_eq!(
        live.ended(),
        [RaffleId::new(&raffle.id), RaffleId::new(other)]
    );
}

/// The housekeeping run ends the live channel of raffles finished over a day
/// ago, once.
#[tokio::test]
async fn housekeeping_ends_stale_live_channels() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let old = finished_raffle(&app).await;
    // A day and a minute later, another finishes.
    app.clock
        .advance(service::housekeeping::CHANNEL_GRACE_SECS + 60);
    let recent = finished_raffle(&app).await;

    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.channels_ended, 1);
    assert_eq!(live.ended(), [RaffleId::new(&old.id)]);
    let r = raffles::get(&app.db, &old.id).await.unwrap().unwrap();
    assert!(r.live_ended_at.is_some());
    assert!(
        raffles::get(&app.db, &recent.id)
            .await
            .unwrap()
            .unwrap()
            .live_ended_at
            .is_none()
    );
    assert!(
        entrants::list(&app.db, &old.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased),
        "housekeeping erases nobody"
    );
    assert_eq!(
        service::housekeeping::run(&app)
            .await
            .unwrap()
            .channels_ended,
        0,
        "once"
    );
}
