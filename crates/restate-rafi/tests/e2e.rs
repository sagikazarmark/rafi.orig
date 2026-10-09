//! Deploys the `Raffle` object to a real `restate-server` and drives a whole
//! raffle through the ingress: freeze, deduplicated draws, a no-show and a
//! redraw, terminal errors, finish (state cleared), and an independent
//! verification of the projected log.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::time::Duration;

use rafi_core::{
    commit::{CommitDoc, UnitRef},
    engine::Command,
    state::{DrawState, Event, Outcome, UnitStatus},
    verify::verify,
};
use rafi_drand::{DrandClient, DrandConfig};
use rafi_platform::{MemoryDrawStore, Recording};
use restate_e2e_harness::{Call, ReusePolicy, ServerSpec, launcher_or_skip};
use restate_rafi::{Raffle, SERVICE_NAME};
use restate_sdk::endpoint::Endpoint;
use serde_json::{Value, json};

const SERVER: ServerSpec = ServerSpec {
    name: "rafi",
    features: &[],
    env: &[],
};

const RAFFLE: &str = "e2e-raffle";

fn doc(entrants: usize) -> CommitDoc {
    CommitDoc::new(
        RAFFLE,
        &rafi_drand::mock_chain().chain().hash(),
        (0..entrants).map(|i| format!("{i:032x}")).collect(),
        vec![
            UnitRef {
                unit_id: "ticket-1".into(),
                item_id: "ticket".into(),
                item_name: "KCD Budapest ticket".into(),
            },
            UnitRef {
                unit_id: "voucher-1".into(),
                item_id: "voucher".into(),
                item_name: "CNCF exam voucher".into(),
            },
        ],
    )
}

/// Waits until the latest projection satisfies `f`.
async fn projected(store: &MemoryDrawStore, f: impl Fn(&DrawState) -> bool) -> DrawState {
    for _ in 0..200 {
        if let Some(s) = store.latest(RAFFLE)
            && f(&s)
        {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "projection not reached; latest: {:#?}",
        store.latest(RAFFLE)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn e2e_raffle() {
    let Some(launcher) = launcher_or_skip(ReusePolicy::Never) else {
        return;
    };
    let restate = launcher.launch(&SERVER).await;

    // Mock drand with no lead time: every round is "already published".
    let drand = DrandClient::new(DrandConfig {
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
        ..DrandConfig::default()
    })
    .unwrap();
    let store = MemoryDrawStore::default();
    let notifier = std::sync::Arc::new(Recording::default());
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, store.clone(), notifier.clone()))
                .build(),
        )
        .await;

    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);
    let cmd = |c: Command| serde_json::to_value(c).unwrap();

    // Freeze; the same list again is a no-op, a different one a conflict.
    let doc = doc(5);
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(doc)), Some("freeze-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let frozen = projected(&store, |s| s.events.is_empty()).await;
    assert_eq!(frozen.entrants.len(), 5);
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(doc)), None)
        .await;
    assert_eq!(reply.status, 200, "refreezing the same list is idempotent");
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(self::doc(4))), None)
        .await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    // A commit to another drand chain than the engine's is refused.
    let mut quicknet = self::doc(5);
    quicknet.raffle_id = "other-raffle".into();
    quicknet.drand_chain = rafi_core::drand::QUICKNET_CHAIN.into();
    let reply = restate
        .invoke(
            &Call::object(SERVICE_NAME, "other-raffle", "freeze"),
            Some(&json!(quicknet)),
            None,
        )
        .await;
    assert_eq!(reply.status, 409, "{}", reply.body);

    // Draw; the same idempotency key twice draws once.
    let draw = cmd(Command::Draw {
        unit_id: "ticket-1".into(),
        offline: false,
    });
    let sent = restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    assert!(sent.status < 300, "{}", sent.body);
    let again = restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    assert!(again.status < 300, "{}", again.body);
    let s = projected(&store, |s| {
        s.unit("ticket-1")
            .is_some_and(|u| u.status == UnitStatus::Drawn)
    })
    .await;
    let announces = s
        .events
        .iter()
        .filter(|e| matches!(e, Event::Announce { .. }))
        .count();
    assert_eq!(announces, 1, "the idempotency key deduplicated the draw");
    let first = s.unit("ticket-1").unwrap().entrant_id.clone().unwrap();

    // The draw journals the clock, the projections (each followed by the live
    // channel's best-effort notify) and the beacon under stable names. The
    // projection above can be seen before the last `notify` is journaled, so
    // wait for the draw to complete (the journal is retained).
    restate
        .admin()
        .await_status(sent.invocation_id(), &["completed"])
        .await;
    assert_eq!(
        restate.admin().runs(sent.invocation_id()).await,
        [
            restate_drand::CLOCK_STEP,
            "project",
            "notify",
            restate_drand::CLOCK_STEP,
            restate_drand::FETCH_STEP,
            "project",
            "notify"
        ]
    );

    // Not here: that person is out and the unit is drawn again.
    let reply = restate
        .invoke(
            &call("resolve"),
            Some(&cmd(Command::Resolve {
                unit_id: "ticket-1".into(),
                outcome: Outcome::NotHere,
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-2"))
        .await;
    let s = projected(&store, |s| {
        s.unit("ticket-1")
            .is_some_and(|u| u.status == UnitStatus::Drawn)
            && s.next_k == 3
    })
    .await;
    assert_ne!(
        s.unit("ticket-1").unwrap().entrant_id.as_deref(),
        Some(first.as_str())
    );
    let claim = cmd(Command::Resolve {
        unit_id: "ticket-1".into(),
        outcome: Outcome::Claimed,
    });
    assert_eq!(
        restate
            .invoke(&call("resolve"), Some(&claim), None)
            .await
            .status,
        200
    );

    // A rejected command is terminal, with a status the caller can show.
    let reply = restate.invoke(&call("resolve"), Some(&claim), None).await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    let reply = restate
        .invoke(
            &call("draw"),
            Some(&cmd(Command::Draw {
                unit_id: "nope".into(),
                offline: false,
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 404, "{}", reply.body);

    // Withdraw the voucher, finish; the object forgets everything.
    let reply = restate
        .invoke(
            &call("resolve"),
            Some(&cmd(Command::Void {
                unit_id: "voucher-1".into(),
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(
        restate
            .invoke(&call("resolve"), Some(&cmd(Command::Finish)), None)
            .await
            .status,
        200
    );
    let finished = projected(&store, |s| s.finished).await;
    let reply = restate.invoke(&call("get"), None, None).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.body, Value::Null, "state is cleared after finishing");
    assert!(
        notifier.notified().iter().all(|id| id == RAFFLE) && notifier.notified().len() >= 4,
        "every write notifies clients: {:?}",
        notifier.notified()
    );

    // The projected log verifies independently.
    let report = verify(
        &doc.canonical_json(),
        &finished.list_hash,
        &finished.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), 2);
}

/// A beacon that cannot be had stalls the draw on its round: a retry (a new
/// invocation) re-enters the same pending draw, and only an explicit abandon
/// ends it, which the verifier reports.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn e2e_stalled_draw() {
    let Some(launcher) = launcher_or_skip(ReusePolicy::Never) else {
        return;
    };
    let restate = launcher.launch(&SERVER).await;
    // quicknet behind an unreachable relay: every fetch fails, and gives up after a second.
    let drand = DrandClient::new(DrandConfig {
        relays: vec!["http://127.0.0.1:9".into()],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    })
    .unwrap();
    let store = MemoryDrawStore::default();
    let notifier = std::sync::Arc::new(Recording::default());
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, store.clone(), notifier.clone()))
                .build(),
        )
        .await;
    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);
    let cmd = |c: Command| serde_json::to_value(c).unwrap();
    let mut doc = doc(5);
    doc.drand_chain = rafi_core::drand::QUICKNET_CHAIN.into();
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(doc)), Some("freeze-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);

    let stalls = |s: &DrawState| {
        s.events
            .iter()
            .filter(|e| matches!(e, Event::Stall { .. }))
            .count()
    };
    let draw = cmd(Command::Draw {
        unit_id: "ticket-1".into(),
        offline: false,
    });
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    let s = projected(&store, |s| stalls(s) == 1).await;
    let pending = s.pending.clone().expect("still pending");

    // Another unit is refused; this one retries the same round.
    let other = cmd(Command::Draw {
        unit_id: "voucher-1".into(),
        offline: false,
    });
    let reply = restate.invoke(&call("draw"), Some(&other), None).await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-2"))
        .await;
    let s = projected(&store, |s| stalls(s) == 2).await;
    assert_eq!(s.pending.as_ref(), Some(&pending));

    // Abandon: logged, the unit is free again.
    let reply = restate
        .invoke(
            &call("resolve"),
            Some(&cmd(Command::Abandon {
                unit_id: "ticket-1".into(),
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let s = projected(&store, |s| s.pending.is_none()).await;
    let report = verify(
        &doc.canonical_json(),
        &s.list_hash,
        &s.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.replaced.len(), 1);
    assert_eq!(report.replaced[0].round, pending.round);
}

/// Cancelling a draw in Restate fails the invocation but records a stall on
/// the same round first, so the organizer gets the controls right away.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs RESTATE_SERVER_BIN"]
async fn e2e_cancelled_draw_stalls() {
    let Some(launcher) = launcher_or_skip(ReusePolicy::Never) else {
        return;
    };
    let restate = launcher.launch(&SERVER).await;
    // The committed round is two minutes away: the draw sleeps durably.
    let drand = DrandClient::new(DrandConfig {
        mock: true,
        lead_secs: 120,
        timeout_secs: 5,
        ..DrandConfig::default()
    })
    .unwrap();
    let store = MemoryDrawStore::default();
    let notifier = std::sync::Arc::new(Recording::default());
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, store.clone(), notifier.clone()))
                .build(),
        )
        .await;
    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);
    let doc = doc(5);
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(doc)), Some("freeze-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);

    let draw = serde_json::to_value(Command::Draw {
        unit_id: "ticket-1".into(),
        offline: false,
    })
    .unwrap();
    let sent = restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    assert!(sent.status < 300, "{}", sent.body);
    let id = sent.invocation_id().to_owned();
    let pending = projected(&store, |s| s.pending.is_some())
        .await
        .pending
        .unwrap();
    // Wait until the draw sleeps towards its round, then cancel it.
    for _ in 0..300 {
        let journal = restate.admin().journal(&id).await;
        if journal.iter().any(|e| e.entry_type.contains("Sleep")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    restate.admin().cancel(&id).await;

    let s = projected(&store, |s| {
        s.events.iter().any(|e| matches!(e, Event::Stall { .. }))
    })
    .await;
    assert_eq!(s.pending.as_ref(), Some(&pending), "same draw, same round");
    assert!(
        s.events.iter().any(
            |e| matches!(e, Event::Stall { reason, round, .. } if reason == "draw cancelled" && *round == pending.round)
        ),
        "{:#?}",
        s.events
    );
    restate.admin().await_status(&id, &["completed"]).await;
    let failure = restate
        .admin()
        .invocation(&id)
        .await
        .completion_failure
        .unwrap_or_default();
    assert!(failure.contains("cancelled"), "{failure:?}");
    // The object still holds the pending draw: a retry re-enters it.
    let reply = restate.invoke(&call("get"), None, None).await;
    let state: DrawState = serde_json::from_value(reply.body).unwrap();
    assert_eq!(state.pending, Some(pending));
}
