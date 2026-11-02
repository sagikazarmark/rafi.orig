//! The views: the full view, the phones' snapshot, the entrants' summary.

use super::*;

pub(in crate::conformance) async fn reads_the_full_view(db: &Db) -> Outcome {
    check(
        Full::read(db, "nope").await.map_err(fail)?.is_none(),
        "no such raffle",
    )?;
    raffle_in(db, "r1", RaffleState::Open).await?;
    let prize = |name, position| prizes::PrizeFields {
        name,
        sponsor: "",
        description: "",
        qty: 1,
        position,
        delivery: Delivery::Later,
    };
    prizes::insert(db, "i2", "r1", &prize("Shirt", 1))
        .await
        .map_err(fail)?;
    prizes::insert(db, "i1", "r1", &prize("Book", 0))
        .await
        .map_err(fail)?;
    prizes::insert(db, "i0", "r1", &prize("Mug", 1))
        .await
        .map_err(fail)?;
    check(
        prizes::update(
            db,
            "i2",
            "r1",
            &prizes::PrizeFields {
                delivery: Delivery::OnTheSpot,
                ..prize("T-shirt", 1)
            },
        )
        .await
        .map_err(fail)?,
        "updating a prize",
    )?;
    check(
        !prizes::update(db, "i9", "r1", &prize("Hat", 2))
            .await
            .map_err(fail)?,
        "updating a missing prize",
    )?;
    join(db, "r1", "z", "z@x", 10).await?;
    test_mode_on(db, "r1").await?;
    entrants::insert_fakes(
        db,
        "r1",
        5,
        &[entrants::Fake {
            id: "y".into(),
            display_name: "Yves".into(),
            claim_code: "Y1".into(),
        }],
        CAP,
    )
    .await
    .map_err(fail)?;

    let full = Full::read(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    let prize_ids: Vec<_> = full.prizes.iter().map(|i| i.id.as_str()).collect();
    check(
        prize_ids == ["i1", "i0", "i2"] && full.prizes[2].name == "T-shirt",
        &format!("prizes by position, then ID: {prize_ids:?}"),
    )?;
    let deliveries: Vec<_> = full.prizes.iter().map(|i| i.delivery).collect();
    check(
        deliveries == [Delivery::Later, Delivery::Later, Delivery::OnTheSpot],
        &format!("how each prize is delivered: {deliveries:?}"),
    )?;
    let entrant_ids: Vec<_> = full.entrants.iter().map(|e| e.id.as_str()).collect();
    check(
        entrant_ids == ["y", "z"],
        &format!("entrants in the order they entered: {entrant_ids:?}"),
    )?;
    check(
        full.raffle.version == raffle(db, "r1").await?.version && full.raffle.version > 0,
        "the raffle's version",
    )?;
    check(
        prizes::list(db, "r1").await.map_err(fail)?.len() == 3,
        "the same prizes",
    )?;
    prizes::delete(db, "i1", "r1").await.map_err(fail)?;
    check(
        Full::read(db, "r1")
            .await
            .map_err(fail)?
            .is_some_and(|f| f.prizes.len() == 2 && f.raffle.version > full.raffle.version),
        "a deleted prize is gone, and the version moved",
    )
}

pub(in crate::conformance) async fn phones_see_only_drawn_names(db: &Db) -> Outcome {
    frozen(db, "r1").await?;
    let (anna, bence) = ("a".repeat(32), "b".repeat(32));
    force_state(db, "r1", RaffleState::Closed).await?;
    for id in [&anna, &bence] {
        add_one(db, "r1", id).await.map_err(fail)?;
    }
    force_state(db, "r1", RaffleState::Frozen).await?;
    patch(
        db,
        "r1",
        &anna,
        entrants::Patch {
            display_name: Some("Anna"),
            ..Default::default()
        },
    )
    .await?;
    let bence_hidden = entrants::Patch {
        display_name: Some("Bence"),
        hidden: Some(true),
        ..Default::default()
    };
    patch(db, "r1", &bence, bence_hidden).await?;
    // Bence drawn, as the engine logged it.
    let draw = Event::Drawn {
        k: 1,
        item: "u1".into(),
        round: 1,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 2,
        index: 0,
        entrant: bence.clone().into(),
    };
    let state =
        Raffle::restore(&manifest("r1", [anna.clone(), bence.clone()]), &[draw]).map_err(fail)?;
    check(state.version() == 2, "one event")?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "writing the draw",
    )?;

    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(snap.state == RaffleState::Drawing, "drawing")?;
    check(
        snap.names.len() == 1 && snap.names.get(&bence) == Some(&None),
        "only the drawn entrant, and their hidden name isn't read",
    )?;
    check(
        snap.drawn.as_ref().is_some_and(|d| d.draws().len() == 1),
        "the raffle as drawn",
    )?;
    check(
        snap.version == raffle(db, "r1").await?.version as u64,
        "the raffle's version",
    )?;

    // Bence isn't here; the prize goes to Anna. Everyone drawn is named,
    // forfeits too.
    let mut state = state;
    state
        .decide(&Decision::Resolve {
            k: None,
            item: "u1".into(),
            resolution: Resolution::NotHere,
        })
        .map_err(fail)?;
    let state = Raffle::restore(
        state.manifest(),
        &[
            state.events(),
            &[Event::Drawn {
                k: 2,
                item: "u1".into(),
                round: 2,
                signature: String::new(),
                randomness: String::new(),
                pass: 1,
                pool_size: 1,
                index: 0,
                entrant: anna.clone().into(),
            }],
        ]
        .concat(),
    )
    .map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "writing the forfeit and the redraw",
    )?;
    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        snap.names.len() == 2
            && snap.names.get(&bence) == Some(&None)
            && snap.names.get(&anna) == Some(&Some("Anna".to_owned())),
        &format!("both drawn entrants: {:?}", snap.names),
    )?;
    check(snap.drawn.as_ref() == Some(&state), "the raffle as drawn")?;

    // Once erased, a drawn entrant has no name to show.
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some(&anna), 0)
        .await
        .map_err(fail)?;
    let snap = live::snapshot(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    check(
        snap.names.len() == 1 && snap.names.get(&bence) == Some(&None),
        &format!("the erased entrant's name isn't read: {:?}", snap.names),
    )?;
    check(
        live::snapshot(db, "nope").await.map_err(fail)?.is_none(),
        "no such raffle",
    )
}

/// What the screens poll ([`Lite`]) summarizes the entrants as the admin's
/// read of every row does ([`Summary::of`]): the count, the latest names
/// (newest first, an add's rows backwards, neither hidden, erased nor too
/// new), the drawn entrants and whether a list was added.
pub(in crate::conformance) async fn summarizes_the_entrants_as_the_rows_do(db: &Db) -> Outcome {
    check(
        Lite::read(db, "nope", NOW).await.map_err(fail)?.is_none(),
        "no such raffle",
    )?;
    let (anna, bence) = ("a".repeat(32), "b".repeat(32));
    frozen(db, "r1").await?;
    force_state(db, "r1", RaffleState::Closed).await?;
    let list = [
        (anna.as_str(), None, None),
        (bence.as_str(), None, None),
        ("cecil", None, None),
        ("dora", None, None),
    ];
    check(
        add_at(db, "r1", "list", Origin::List, &list, CAP, NOW - 100)
            .await
            .map_err(fail)?,
        "the list",
    )?;
    add_at(
        db,
        "r1",
        "b-eve",
        Origin::One,
        &[("eve", None, None)],
        CAP,
        NOW - 50,
    )
    .await
    .map_err(fail)?;
    add_at(
        db,
        "r1",
        "b-fay",
        Origin::One,
        &[("fay", None, None)],
        CAP,
        NOW,
    )
    .await
    .map_err(fail)?;
    patch(
        db,
        "r1",
        "dora",
        entrants::Patch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await?;
    force_state(db, "r1", RaffleState::Frozen).await?;
    let draw = Event::Drawn {
        k: 1,
        item: "u1".into(),
        round: 1,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 2,
        index: 0,
        entrant: bence.clone().into(),
    };
    let state =
        Raffle::restore(&manifest("r1", [anna.clone(), bence.clone()]), &[draw]).map_err(fail)?;
    check(
        save_raffle(db, &state, NOW).await.map_err(fail)?,
        "writing the draw",
    )?;
    force_state(db, "r1", RaffleState::Finished).await?;
    entrants::erase(db, "r1", Some("cecil"), 0)
        .await
        .map_err(fail)?;

    let names_before = NOW - 5;
    let lite = Lite::read(db, "r1", names_before)
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    let full = Full::read(db, "r1")
        .await
        .map_err(fail)?
        .ok_or("the raffle is gone")?;
    let drawn: Vec<String> = full
        .drawn
        .as_ref()
        .map(|s| {
            s.draws()
                .into_iter()
                .map(|d| d.entrant.to_string())
                .collect()
        })
        .unwrap_or_default();
    let of_rows = Summary::of(
        &full.entrants,
        names_before,
        drawn.iter().map(String::as_str),
    );
    check(
        lite.entrants == of_rows,
        &format!("read: {:?}\nof the rows: {of_rows:?}", lite.entrants),
    )?;
    let summary = lite.entrants;
    check(
        (summary.count, summary.unerased) == (6, 5),
        &format!("counted: {} and {}", summary.count, summary.unerased),
    )?;
    check(
        summary.recent_names == ["eve", bence.as_str(), anna.as_str()],
        &format!(
            "newest first, without the hidden, the erased and the too new: {:?}",
            summary.recent_names
        ),
    )?;
    check(
        summary.drawn.len() == 1
            && summary.drawn[0].id == bence
            && summary.drawn[0].source == Source::Manual,
        &format!("the drawn: {:?}", summary.drawn),
    )?;
    check(summary.list_added, "a list was added")?;
    check(
        lite.raffle.version == full.raffle.version && lite.drawn == full.drawn,
        "the same raffle as drawn",
    )?;
    check(lite.prizes.len() == full.prizes.len(), "the same prizes")
}
