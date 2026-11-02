//! Prizes' delivery: on the spot with the claim, or later, as each claim keeps
//! it.

use super::*;

/// Claiming an item of a prize handed over on the spot delivers it, in the
/// save that stores the claim: a book handed over, a voucher sent later.
/// Only the claim does (not the draw, not a forfeit), only once, never for an
/// erased winner, and a stale save delivers nothing.
pub(in crate::conformance) async fn delivers_on_the_spot_with_the_claim(db: &Db) -> Outcome {
    let (anna, bence, csilla) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
    let manifest = Manifest::new(
        "r1",
        BeaconSource::QUICKNET,
        vec![
            anna.clone().into(),
            bence.clone().into(),
            csilla.clone().into(),
        ],
        vec![
            ManifestItem {
                id: "book-1".into(),
                name: "Book".into(),
            },
            ManifestItem {
                id: "voucher-1".into(),
                name: "Voucher".into(),
            },
        ],
    );
    frozen_with(db, &manifest).await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence, &csilla] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    for (id, name, delivery) in [
        ("book", "Book", Delivery::OnTheSpot),
        ("voucher", "Voucher", Delivery::Later),
    ] {
        let fields = prizes::PrizeFields {
            name,
            sponsor: "",
            description: "",
            qty: 1,
            position: 0,
            delivery,
        };
        prizes::insert(db, id, "r1", &fields).await.map_err(fail)?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    let drawn = |k, item: &str, entrant: &str| Event::Drawn {
        k,
        item: item.into(),
        round: k,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 3,
        index: 0,
        entrant: entrant.to_owned().into(),
    };
    let resolved = |k, resolution| Event::Resolved { k, resolution };
    let delivered_at = |id: String| async move {
        entrants::get(db, "r1", &id)
            .await
            .map_err(fail)?
            .ok_or_else(|| format!("entrant {id} is gone"))
            .map(|e| e.delivered_at)
    };
    let save = |events: Vec<Event>, now: i64| {
        let manifest = manifest.clone();
        async move {
            let state = Raffle::restore(&manifest, &events).map_err(fail)?;
            save_raffle(db, &state, now).await.map_err(fail)
        }
    };

    // Bence is drawn for the book and isn't here; Anna is drawn instead.
    let mut events = vec![drawn(1, "book-1", &bence)];
    check(save(events.clone(), NOW).await?, "the first draw")?;
    check(
        delivered_at(bence.clone()).await?.is_none(),
        "a draw delivers nothing",
    )?;
    events.extend([resolved(1, Resolution::NotHere), drawn(2, "book-1", &anna)]);
    check(
        save(events.clone(), NOW).await?,
        "the forfeit and the redraw",
    )?;
    check(
        delivered_at(bence.clone()).await?.is_none(),
        "a forfeit delivers nothing",
    )?;

    // Anna claims the book; Csilla, drawn for the voucher, claims it.
    let stale = events.clone();
    events.extend([
        resolved(2, Resolution::Claimed),
        drawn(3, "voucher-1", &csilla),
        resolved(3, Resolution::Claimed),
    ]);
    check(save(events.clone(), NOW + 5).await?, "the claims")?;
    check(
        delivered_at(anna.clone()).await? == Some(NOW + 5),
        "the book left with Anna when she claimed it",
    )?;
    check(
        delivered_at(csilla.clone()).await?.is_none(),
        "the voucher is still to be sent",
    )?;
    check(!save(stale, NOW + 9).await?, "a stale save is refused")?;

    // Undelivered by hand, it stays so: the claim isn't stored again.
    patch(
        db,
        "r1",
        &anna,
        entrants::Patch {
            delivered_at: Some(None),
            ..Default::default()
        },
    )
    .await?;
    events.push(Event::Finished);
    check(save(events.clone(), NOW + 9).await?, "the finish")?;
    check(
        delivered_at(anna.clone()).await?.is_none(),
        "a later save doesn't deliver again",
    )?;

    // The same evening in one save, the winner of the book erased first.
    drop_raffle(db, "r1").await?;
    frozen_with(db, &manifest).await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence, &csilla] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    let fields = prizes::PrizeFields {
        name: "Book",
        sponsor: "",
        description: "",
        qty: 1,
        position: 0,
        delivery: Delivery::OnTheSpot,
    };
    prizes::insert(db, "book", "r1", &fields)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some(&anna), NOW)
        .await
        .map_err(fail)?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    check(save(events, NOW).await?, "the evening at once")?;
    check(
        delivered_at(anna).await?.is_none(),
        "an erased winner isn't delivered to",
    )?;
    delivers_once_every_claimed_prize_is(db).await
}

/// More items than people: Dora alone claims a book handed over on the
/// spot, then in pass 2 a voucher sent later, then a mug handed over. She is
/// delivered only while every prize she claimed is, saved claim by claim or
/// at once.
async fn delivers_once_every_claimed_prize_is(db: &Db) -> Outcome {
    let dora = "d".repeat(32);
    let manifest = Manifest::new(
        "r2",
        BeaconSource::QUICKNET,
        vec![dora.clone().into()],
        ["book", "voucher", "mug"]
            .map(|p| ManifestItem {
                id: format!("dora-{p}-1").into(),
                name: p.into(),
            })
            .into(),
    );
    let setup = || async {
        frozen_with(db, &manifest).await?;
        force_state(db, "r2", RaffleState::Closed).await?;
        add_one(db, "r2", &dora).await.map_err(fail)?;
        for (id, delivery) in [
            ("book", Delivery::OnTheSpot),
            ("voucher", Delivery::Later),
            ("mug", Delivery::OnTheSpot),
        ] {
            let fields = prizes::PrizeFields {
                name: id,
                sponsor: "",
                description: "",
                qty: 1,
                position: 0,
                delivery,
            };
            prizes::insert(db, &format!("dora-{id}"), "r2", &fields)
                .await
                .map_err(fail)?;
        }
        force_state(db, "r2", RaffleState::Frozen).await
    };
    let claim = |k: u64, item: &str| {
        [
            Event::Drawn {
                k,
                item: format!("dora-{item}-1").into(),
                round: k,
                signature: String::new(),
                randomness: String::new(),
                pass: if k == 1 { 1 } else { 2 },
                pool_size: 1,
                index: 0,
                entrant: dora.clone().into(),
            },
            Event::Resolved {
                k,
                resolution: Resolution::Claimed,
            },
        ]
    };
    let delivered =
        || async { Ok::<_, String>(entrant(db, "r2", &dora).await?.delivered_at.is_some()) };

    setup().await?;
    let mut events = Vec::new();
    for (k, item, want, what) in [
        (1, "book", true, "the book was handed over"),
        (2, "voucher", false, "the voucher is still to be sent"),
        (
            3,
            "mug",
            false,
            "the mug was handed over, the voucher still isn't",
        ),
    ] {
        events.extend(claim(k, item));
        let state = Raffle::restore(&manifest, &events).map_err(fail)?;
        check(
            save_raffle(db, &state, NOW).await.map_err(fail)?,
            &format!("saving the {item}"),
        )?;
        check(delivered().await? == want, what)?;
    }

    drop_raffle(db, "r2").await?;
    setup().await?;
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "saving it at once",
    )?;
    check(!delivered().await?, "at once, the voucher still isn't")
}

/// A claim keeps the delivery its prize had when it was claimed: Eve claims
/// a voucher sent later; the organizer then switches the voucher to on the
/// spot (for the copies still to come, they think); Eve claims a mug handed
/// over. She is still owed the voucher, so she isn't delivered, and the
/// voucher's draw still reads `later`.
pub(in crate::conformance) async fn a_claim_keeps_its_delivery(db: &Db) -> Outcome {
    let eve = "e".repeat(32);
    let manifest = Manifest::new(
        "r1",
        BeaconSource::QUICKNET,
        vec![eve.clone().into()],
        ["voucher", "mug"]
            .map(|p| ManifestItem {
                id: format!("{p}-1").into(),
                name: p.into(),
            })
            .into(),
    );
    frozen_with(db, &manifest).await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    add_one(db, "r1", &eve).await.map_err(fail)?;
    for (id, delivery) in [("voucher", Delivery::Later), ("mug", Delivery::OnTheSpot)] {
        let fields = prizes::PrizeFields {
            name: id,
            sponsor: "",
            description: "",
            qty: 1,
            position: 0,
            delivery,
        };
        prizes::insert(db, id, "r1", &fields).await.map_err(fail)?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    let claim = |k: u64, item: &str| {
        [
            Event::Drawn {
                k,
                item: format!("{item}-1").into(),
                round: k,
                signature: String::new(),
                randomness: String::new(),
                pass: if k == 1 { 1 } else { 2 },
                pool_size: 1,
                index: 0,
                entrant: eve.clone().into(),
            },
            Event::Resolved {
                k,
                resolution: Resolution::Claimed,
            },
        ]
    };
    let draw_delivery = |k: u64| async move {
        db.query("SELECT delivery FROM draws WHERE raffle_id = 'r1' AND k = ?")
            .bind(k)
            .one::<(Option<String>,)>()
            .await
            .map(|(d,)| d)
            .map_err(fail)
    };

    let mut events = claim(1, "voucher").to_vec();
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "saving the voucher's claim",
    )?;
    check(
        draw_delivery(1).await?.as_deref() == Some("later"),
        "the voucher's claim is recorded as sent later",
    )?;
    check(
        prizes::set_delivery(db, "voucher", "r1", Delivery::OnTheSpot)
            .await
            .map_err(fail)?,
        "the voucher switched to on the spot",
    )?;
    events.extend(claim(2, "mug"));
    let state = Raffle::restore(&manifest, &events).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW + 5).await.map_err(fail)?,
        "saving the mug's claim",
    )?;
    check(
        entrant(db, "r1", &eve).await?.delivered_at.is_none(),
        "the voucher she claimed is still owed: not delivered",
    )?;
    check(
        draw_delivery(1).await?.as_deref() == Some("later"),
        "the voucher's claim still reads sent later",
    )?;
    check(
        draw_delivery(2).await?.as_deref() == Some("on_the_spot"),
        "the mug's claim is recorded as handed over",
    )
}
