//! Raffles' settings and lifecycle: the freeze's steps, the locked rules, test
//! mode, registration, owner defaults, what a freeze fixes, deleting.

use super::*;

pub(in crate::conformance) async fn freezes_in_steps(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Draft).await?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "a raffle that isn't in the state it froze from",
    )?;
    check(
        raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "opening",
    )?;
    check(
        !raffles::transition(db, "r1", RaffleState::Draft, RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "a transition from a state the raffle has left",
    )?;
    for id in ["a", "b", "c"] {
        check(join(db, "r1", id, &format!("{id}@x"), 10).await?, "joining")?;
    }
    check(
        !entrants::erase(db, "r1", Some("c"), 0)
            .await
            .map_err(fail)?,
        "nobody is erased while the raffle runs",
    )?;
    check(
        entrants::count_active(db, "r1").await.map_err(fail)? == 3,
        "everyone counts",
    )?;

    check(
        raffles::freeze(db, "r1", RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "freezing",
    )?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "freezing twice",
    )?;
    check(
        !join(db, "r1", "d", "d@x", 10).await?,
        "no joins once frozen",
    )?;
    let mut ids = entrants::active_ids(db, "r1").await.map_err(fail)?;
    ids.sort();
    check(
        ids == ["a", "b", "c"],
        &format!("the entrants for the manifest: {ids:?}"),
    )?;

    check(
        raffles::set_manifest(db, "r1", "{}", "h1", NOW)
            .await
            .map_err(fail)?,
        "writing the manifest",
    )?;
    check(
        !raffles::set_manifest(db, "r1", "{}", "h2", NOW)
            .await
            .map_err(fail)?,
        "writing the manifest again",
    )?;
    raffles::unfreeze(db, "r1", RaffleState::Closed, NOW)
        .await
        .map_err(fail)?;
    let r = raffle(db, "r1").await?;
    check(
        r.state == RaffleState::Frozen
            && r.manifest_hash.as_deref() == Some("h1")
            && r.manifest.is_some()
            && r.frozen_at.is_some(),
        "the first commit stays, and a committed raffle stays frozen",
    )?;

    // A freeze that found nobody to draw from goes back.
    raffle_in(db, "r2", RaffleState::Closed).await?;
    check(
        raffles::freeze(db, "r2", RaffleState::Closed, true, NOW)
            .await
            .map_err(fail)?,
        "freezing a closed raffle",
    )?;
    raffles::unfreeze(db, "r2", RaffleState::Closed, NOW)
        .await
        .map_err(fail)?;
    check(
        raffle(db, "r2").await?.state == RaffleState::Closed,
        "unfrozen",
    )
}

/// The rules change only until an entrant locks them (fakes don't); the
/// other settings always do.
pub(in crate::conformance) async fn rules_are_fixed_once_an_entrant_locks_them(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let save = async |s: &raffles::Settings<'_>| {
        raffles::update_settings(db, "r1", s, NOW)
            .await
            .map(|w| w.written)
    };
    check(
        save(&settings_with("one")).await.map_err(fail)?,
        "no entrants",
    )?;
    let fake = entrants::Fake {
        id: "f1".into(),
        display_name: "Fake".into(),
        claim_code: "F001".into(),
    };
    entrants::insert_fakes(db, "r1", 0, &[fake], CAP)
        .await
        .map_err(fail)?;
    check(
        save(&settings_with("two")).await.map_err(fail)?,
        "only fakes",
    )?;
    check(
        add_one(db, "r1", "m1").await.map_err(fail)?,
        "adding someone",
    )?;
    check(
        !save(&settings_with("three")).await.map_err(fail)?,
        "accepted: the rules are fixed",
    )?;
    // So are the event date, the period, and the privacy text and link.
    for (what, fixed) in [
        (
            "the event date",
            raffles::Settings {
                event_date: "2100-01-02",
                event_ends_at: FAR + 86_400,
                ..settings_with("two")
            },
        ),
        (
            "the period",
            raffles::Settings {
                retention_days: 3,
                ..settings_with("two")
            },
        ),
        (
            "the privacy text",
            raffles::Settings {
                privacy_text: "ours",
                ..settings_with("two")
            },
        ),
        (
            "the privacy link",
            raffles::Settings {
                privacy_url: "https://x",
                ..settings_with("two")
            },
        ),
    ] {
        check(
            !save(&fixed).await.map_err(fail)?,
            &format!("accepted: {what} is fixed"),
        )?;
    }
    let renamed = raffles::Settings {
        title: "renamed",
        privacy_email: "dpo@x",
        ..settings_with("two")
    };
    check(
        save(&renamed).await.map_err(fail)?,
        "the same fixed settings, another title and privacy contact",
    )?;
    let r = raffle(db, "r1").await?;
    check(
        r.rules == "two" && r.title == "renamed",
        &format!("stored {:?}, {:?}", r.rules, r.title),
    )
}

/// Fakes only go into a raffle in test mode; switching it off removes them in
/// the same write, and test mode stays as it is once the list is frozen.
pub(in crate::conformance) async fn fakes_go_with_test_mode(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Open).await?;
    let fake = |id: &str| entrants::Fake {
        id: id.into(),
        display_name: "Fake".into(),
        claim_code: id.to_uppercase(),
    };
    check(
        !entrants::insert_fakes(db, "r1", 0, &[fake("f0")], CAP)
            .await
            .map_err(fail)?,
        "no fakes outside test mode",
    )?;
    let save = async |test_mode: bool| {
        raffles::update_settings(
            db,
            "r1",
            &raffles::Settings {
                test_mode,
                ..settings_with("")
            },
            NOW,
        )
        .await
        .map_err(fail)
    };
    let on = save(true).await?;
    check(
        on.written && !on.fakes_removed,
        &format!("test mode on: {on:?}"),
    )?;
    check(
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")], CAP)
            .await
            .map_err(fail)?,
        "fakes in test mode",
    )?;
    add_one(db, "r1", "m1").await.map_err(fail)?;
    check(
        entrants::count_fakes(db, "r1").await.map_err(fail)? == 2,
        "two fakes",
    )?;
    let fakes = entrants::list(db, "r1").await.map_err(fail)?;
    check(
        fakes
            .iter()
            .filter(|e| e.source == rafi_lifecycle::entrant::Source::Fake)
            .all(|e| e.consent_version.is_none() && e.consent_rules_at.is_none()),
        "fakes consented to nothing",
    )?;

    let off = save(false).await?;
    check(
        off.written && off.fakes_removed,
        &format!("test mode off: {off:?}"),
    )?;
    let ids: Vec<String> = entrants::list(db, "r1")
        .await
        .map_err(fail)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    check(
        ids == ["m1"],
        &format!("the fakes went, the person stayed: {ids:?}"),
    )?;

    // Frozen: test mode no longer changes, so nothing is removed.
    save(true).await?;
    entrants::insert_fakes(db, "r1", 0, &[fake("f3")], CAP)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    let off = save(false).await?;
    check(
        !off.written && !off.fakes_removed,
        &format!("test mode off once frozen: {off:?}"),
    )?;
    check(raffle(db, "r1").await?.test_mode, "still in test mode")?;
    check(
        save(true).await?.written,
        "the other settings save once frozen",
    )?;
    check(
        entrants::count_fakes(db, "r1").await.map_err(fail)? == 1,
        "the frozen fake stays",
    )
}

/// Registration (joining with the QR code) guards the lifecycle's moves in
/// SQL: a raffle opens only with it, freezes from Published only without
/// it, and it switches before the freeze but not while registration is open.
pub(in crate::conformance) async fn registration_guards_the_lifecycle(db: &Db) -> Outcome {
    let save = async |id: &str, registration: bool| {
        raffles::update_settings(
            db,
            id,
            &raffles::Settings {
                registration,
                ..settings_with("")
            },
            NOW,
        )
        .await
        .map(|w| w.written)
        .map_err(fail)
    };
    let state = async |id: &str| raffle(db, id).await.map(|r| (r.state, r.registration));

    // Published, with registration: no freeze from there.
    raffle_in(db, "r1", RaffleState::Ready).await?;
    check(raffle(db, "r1").await?.registration, "on by default")?;
    check(
        !raffles::freeze(db, "r1", RaffleState::Ready, false, NOW)
            .await
            .map_err(fail)?,
        "a freeze from Published with registration on",
    )?;
    // Switched off: no opening, and the freeze goes from Published.
    check(save("r1", false).await?, "switching it off when published")?;
    check(
        !raffles::transition(db, "r1", RaffleState::Ready, RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "opening without registration",
    )?;
    check(
        raffles::freeze(db, "r1", RaffleState::Ready, false, NOW)
            .await
            .map_err(fail)?,
        "a freeze from Published without registration",
    )?;
    check(!save("r1", true).await?, "switching it on once frozen")?;
    raffles::unfreeze(db, "r1", RaffleState::Ready, NOW)
        .await
        .map_err(fail)?;
    check(
        state("r1").await? == (RaffleState::Ready, false),
        "back to Published, registration still off",
    )?;

    // Open: it doesn't switch until registration closes.
    raffle_in(db, "r2", RaffleState::Ready).await?;
    check(
        raffles::transition(db, "r2", RaffleState::Ready, RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "opening with registration",
    )?;
    check(!save("r2", false).await?, "switching it off while open")?;
    check(
        save("r2", true).await?,
        "the other settings save while open",
    )?;
    check(
        state("r2").await? == (RaffleState::Open, true),
        "still open, with registration",
    )?;
    check(
        raffles::transition(db, "r2", RaffleState::Open, RaffleState::Closed, true, NOW)
            .await
            .map_err(fail)?,
        "closing",
    )?;
    check(save("r2", false).await?, "switching it off once closed")?;
    check(
        !raffles::transition(db, "r2", RaffleState::Closed, RaffleState::Open, true, NOW)
            .await
            .map_err(fail)?,
        "reopening without registration",
    )?;
    check(
        raffles::freeze(db, "r2", RaffleState::Closed, false, NOW)
            .await
            .map_err(fail)?,
        "a closed raffle switched off freezes from there",
    )?;
    check(
        state("r2").await? == (RaffleState::Frozen, false),
        "frozen without registration",
    )
}

/// An owner's defaults: none until saved, then replaced as a whole; other
/// owners have their own.
pub(in crate::conformance) async fn saves_owner_defaults(db: &Db) -> Outcome {
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "none yet",
    )?;
    let first = defaults::Defaults {
        organizer_name: "Org".into(),
        contact_email: "a@b.c".into(),
        privacy_email: "dpo@b.c".into(),
        privacy_text: "Ours.".into(),
        privacy_url: "https://b.c/p".into(),
        rules: "One.".into(),
        retention_days: Some(3),
        registration: false,
    };
    defaults::save(db, "o", &first, NOW).await.map_err(fail)?;
    let second = defaults::Defaults {
        rules: "Two.".into(),
        retention_days: None,
        registration: true,
        ..first.clone()
    };
    defaults::save(db, "o", &second, NOW).await.map_err(fail)?;
    check(
        defaults::get(db, "o").await.map_err(fail)? == Some(second),
        "the second save replaces the first, an unset period and registration included",
    )?;
    check(
        defaults::get(db, "p").await.map_err(fail)?.is_none(),
        "another owner's",
    )?;
    defaults::delete(db, "o").await.map_err(fail)?;
    check(
        defaults::get(db, "o").await.map_err(fail)?.is_none(),
        "deleted",
    )
}

pub(in crate::conformance) async fn the_list_and_prizes_are_fixed_once_frozen(db: &Db) -> Outcome {
    raffle_in(db, "r1", RaffleState::Draft).await?;
    test_mode_on(db, "r1").await?;
    let prize = |name, qty| prizes::PrizeFields {
        name,
        sponsor: "",
        description: "",
        qty,
        position: 0,
        delivery: Delivery::Later,
    };
    let fake = |id: &str| entrants::Fake {
        id: id.into(),
        display_name: "Fake".into(),
        claim_code: id.to_uppercase(),
    };
    check(
        add_one(db, "r1", "m1").await.map_err(fail)?,
        "added by hand while closed",
    )?;
    check(
        entrants::insert_fakes(db, "r1", 0, &[fake("f1"), fake("f2")], CAP)
            .await
            .map_err(fail)?,
        "fakes while closed",
    )?;
    check(
        prizes::insert(db, "i1", "r1", &prize("Book", 1))
            .await
            .map_err(fail)?,
        "a prize while closed",
    )?;
    check(
        entrants::delete(db, "r1", "f2", None).await.map_err(fail)?,
        "removed while closed",
    )?;

    force_state(db, "r1", RaffleState::Frozen).await?;
    check(
        !add_one(db, "r1", "m2").await.map_err(fail)?,
        "nobody is added by hand once frozen",
    )?;
    check(
        !entrants::insert_fakes(db, "r1", 0, &[fake("f3")], CAP)
            .await
            .map_err(fail)?,
        "no fakes once frozen",
    )?;
    check(
        !entrants::delete(db, "r1", "m1", None).await.map_err(fail)?,
        "nobody is removed once frozen",
    )?;
    check(
        !prizes::insert(db, "i2", "r1", &prize("Mug", 1))
            .await
            .map_err(fail)?,
        "no prize is added once frozen",
    )?;
    check(
        !prizes::update(db, "i1", "r1", &prize("Big book", 1))
            .await
            .map_err(fail)?,
        "no prize changes once frozen",
    )?;
    check(
        !prizes::delete(db, "i1", "r1").await.map_err(fail)?,
        "no prize is removed once frozen",
    )?;
    let mut ids: Vec<String> = entrants::list(db, "r1")
        .await
        .map_err(fail)?
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    check(
        ids == ["f1", "m1"],
        &format!("the list as it was frozen: {ids:?}"),
    )?;
    check(
        prizes::list(db, "r1").await.map_err(fail)?.len() == 1,
        "the prizes as they were frozen",
    )?;

    // How a prize is delivered isn't in the manifest: it changes until the
    // raffle is over.
    for state in [RaffleState::Frozen, RaffleState::Drawing] {
        force_state(db, "r1", state).await?;
        check(
            prizes::set_delivery(db, "i1", "r1", Delivery::OnTheSpot)
                .await
                .map_err(fail)?,
            &format!("the delivery changes while {}", state.as_str()),
        )?;
    }
    check(
        prizes::set_delivery(db, "i1", "r1", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "setting it again counts as a change",
    )?;
    check(
        !prizes::set_delivery(db, "i9", "r1", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "no such prize",
    )?;
    check(
        !prizes::set_delivery(db, "i1", "r2", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "not that raffle's prize",
    )?;
    for state in [RaffleState::Finished, RaffleState::Purged] {
        force_state(db, "r1", state).await?;
        check(
            !prizes::set_delivery(db, "i1", "r1", Delivery::Later)
                .await
                .map_err(fail)?,
            &format!("not once {}", state.as_str()),
        )?;
    }
    check(
        prizes::list(db, "r1").await.map_err(fail)?[0].delivery == Delivery::OnTheSpot,
        "the delivery as last set",
    )
}

/// Deleting a raffle checks, in its own statement, that no draw was
/// announced: a frozen raffle with no draw goes; one whose draw the engine
/// stored after the caller's check (a draw announced, not even settled)
/// stays, with its log.
pub(in crate::conformance) async fn deletes_only_undrawn_raffles(db: &Db) -> Outcome {
    let manifest = manifest("r1", ["a".repeat(32), "b".repeat(32)]);
    frozen_with(db, &manifest).await?;
    let opened = Raffle::open(&manifest).map_err(fail)?;
    check(
        save_raffle(db, &opened, NOW).await.map_err(fail)?,
        "opened by the engine",
    )?;
    check(
        raffles::delete(db, "r1").await.map_err(fail)?,
        "frozen and opened, nothing drawn: deleted",
    )?;
    check(
        raffles::get(db, "r1").await.map_err(fail)?.is_none(),
        "gone",
    )?;

    frozen_with(db, &manifest).await?;
    let mut drawing = Raffle::open(&manifest).map_err(fail)?;
    drawing
        .announce(
            &rafi_raffle::DrawRequest { item: "u1".into() },
            &BeaconSource::QUICKNET,
            1,
            0,
        )
        .map_err(fail)?;
    check(
        save_raffle(db, &drawing, NOW).await.map_err(fail)?,
        "a draw announced",
    )?;
    check(
        !raffles::delete(db, "r1").await.map_err(fail)?,
        "an announced draw keeps the raffle",
    )?;
    check(
        raffles::get(db, "r1").await.map_err(fail)?.is_some(),
        "still there",
    )?;
    let (events, _) = stored_log(db, "r1").await?;
    check(events.len() == 1, "with its log")
}
