//! Deploys the `Drawing` object to a real `restate-server` and drives a whole
//! drawing through the ingress: opening, deduplicated draws, a no-show and a
//! redraw, terminal errors, finish (state cleared), and an independent
//! verification of the projected log.
//!
//! Ignored: run with `RESTATE_SERVER_BIN` set and `-- --ignored`.

#![cfg(unix)]

use std::time::Duration;

use rafi_drand::{DrandClient, DrandConfig};
use rafi_drawing::{
    BeaconSource, Decision, DrawRequest, Drawing, Event, ItemStatus, Manifest, ManifestItem,
    Resolution, verify,
};
use rafi_engine::MemoryDrawStore;
use rafi_engine_restate::SERVICE_NAME;
use rafi_platform::Recording;
use restate_e2e_harness::{Call, ReusePolicy, ServerSpec, launcher_or_skip};
use restate_sdk::endpoint::Endpoint;
use serde_json::{Value, json};

const SERVER: ServerSpec = ServerSpec {
    name: "rafi",
    features: &[],
    env: &[],
};

const RAFFLE: &str = "e2e-raffle";

fn doc(entrants: usize) -> Manifest {
    Manifest::new(
        RAFFLE,
        BeaconSource::Drand(rafi_drand::mock_chain().chain().hash()),
        (0..entrants).map(|i| format!("{i:032x}").into()).collect(),
        vec![
            ManifestItem {
                id: "ticket-1".into(),
                name: "KCD Budapest ticket".into(),
            },
            ManifestItem {
                id: "voucher-1".into(),
                name: "CNCF exam voucher".into(),
            },
        ],
    )
}

/// Waits until the latest projection satisfies `f`.
async fn projected(store: &MemoryDrawStore, f: impl Fn(&Drawing) -> bool) -> Drawing {
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
                .bind(rafi_engine_restate::Drawing::new(
                    drand,
                    store.clone(),
                    rafi_engine::Notify::new(notifier.clone(), rafi_drawing::DrawingId::as_str),
                ))
                .build(),
        )
        .await;

    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);

    // Freeze; the same list again is a no-op, a different one a conflict.
    let doc = doc(5);
    let reply = restate
        .invoke(&call("open"), Some(&json!(doc)), Some("open-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let frozen = projected(&store, |s| s.events().is_empty()).await;
    assert_eq!(frozen.entrants().len(), 5);
    let reply = restate.invoke(&call("open"), Some(&json!(doc)), None).await;
    assert_eq!(
        reply.status, 200,
        "reopening with the same manifest is idempotent"
    );
    let reply = restate
        .invoke(&call("open"), Some(&json!(self::doc(4))), None)
        .await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    // A drawing on another beacon source than the engine's is refused.
    let mut quicknet = self::doc(5);
    quicknet.drawing = "other-raffle".into();
    quicknet.beacon = BeaconSource::QUICKNET;
    let reply = restate
        .invoke(
            &Call::object(SERVICE_NAME, "other-raffle", "open"),
            Some(&json!(quicknet)),
            None,
        )
        .await;
    assert_eq!(reply.status, 409, "{}", reply.body);

    // Draw; the same idempotency key twice draws once.
    let draw = json!(DrawRequest {
        item: "ticket-1".into(),
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
        s.item(&"ticket-1".into())
            .is_some_and(|u| u.status() == ItemStatus::Drawn)
    })
    .await;
    let announces = s
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Announced { .. }))
        .count();
    assert_eq!(announces, 1, "the idempotency key deduplicated the draw");
    let first = s
        .item(&"ticket-1".into())
        .unwrap()
        .winner()
        .cloned()
        .unwrap();

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

    // Not here: that person is out and the item is drawn again.
    let reply = restate
        .invoke(
            &call("decide"),
            Some(&json!(Decision::Resolve {
                item: "ticket-1".into(),
                resolution: Resolution::NotHere,
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-2"))
        .await;
    let s = projected(&store, |s| {
        s.item(&"ticket-1".into())
            .is_some_and(|u| u.status() == ItemStatus::Drawn)
            && s.draws().len() == 2
    })
    .await;
    assert_ne!(s.item(&"ticket-1".into()).unwrap().winner(), Some(&first));
    let claim = json!(Decision::Resolve {
        item: "ticket-1".into(),
        resolution: Resolution::Claimed,
    });
    assert_eq!(
        restate
            .invoke(&call("decide"), Some(&claim), None)
            .await
            .status,
        200
    );

    // A rejected command is terminal, with a status the caller can show.
    let reply = restate.invoke(&call("decide"), Some(&claim), None).await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    let reply = restate
        .invoke(
            &call("draw"),
            Some(&json!(DrawRequest {
                item: "nope".into(),
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 404, "{}", reply.body);

    // Withdraw the voucher, finish; the object forgets everything.
    let reply = restate
        .invoke(
            &call("decide"),
            Some(&json!(Decision::Void {
                item: "voucher-1".into(),
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(
        restate
            .invoke(&call("decide"), Some(&json!(Decision::Finish)), None)
            .await
            .status,
        200
    );
    let finished = projected(&store, |s| s.is_finished()).await;
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
        &finished.manifest_hash().to_string(),
        finished.events(),
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
                .bind(rafi_engine_restate::Drawing::new(
                    drand,
                    store.clone(),
                    rafi_engine::Notify::new(notifier.clone(), rafi_drawing::DrawingId::as_str),
                ))
                .build(),
        )
        .await;
    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);
    let mut doc = doc(5);
    doc.beacon = BeaconSource::QUICKNET;
    let reply = restate
        .invoke(&call("open"), Some(&json!(doc)), Some("open-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);

    let stalls = |s: &Drawing| {
        s.events()
            .iter()
            .filter(|e| matches!(e, Event::Stalled { .. }))
            .count()
    };
    let draw = json!(DrawRequest {
        item: "ticket-1".into(),
    });
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    let s = projected(&store, |s| stalls(s) == 1).await;
    let pending = s.pending().cloned().expect("still pending");

    // Another item is refused; this one retries the same round.
    let other = json!(DrawRequest {
        item: "voucher-1".into(),
    });
    let reply = restate.invoke(&call("draw"), Some(&other), None).await;
    assert_eq!(reply.status, 409, "{}", reply.body);
    restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-2"))
        .await;
    let s = projected(&store, |s| stalls(s) == 2).await;
    assert_eq!(s.pending(), Some(&pending));

    // Abandon: logged, the item is free again.
    let reply = restate
        .invoke(
            &call("decide"),
            Some(&json!(Decision::Abandon {
                item: "ticket-1".into(),
            })),
            None,
        )
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let s = projected(&store, |s| s.pending().is_none()).await;
    let report = verify(
        &doc.canonical_json(),
        &s.manifest_hash().to_string(),
        s.events(),
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.abandoned.len(), 1);
    assert_eq!(report.abandoned[0].round, pending.round);
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
                .bind(rafi_engine_restate::Drawing::new(
                    drand,
                    store.clone(),
                    rafi_engine::Notify::new(notifier.clone(), rafi_drawing::DrawingId::as_str),
                ))
                .build(),
        )
        .await;
    let call = |handler| Call::object(SERVICE_NAME, RAFFLE, handler);
    let doc = doc(5);
    let reply = restate
        .invoke(&call("open"), Some(&json!(doc)), Some("open-1"))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.body);

    let draw = serde_json::to_value(DrawRequest {
        item: "ticket-1".into(),
    })
    .unwrap();
    let sent = restate
        .invoke(&call("draw").send(), Some(&draw), Some("draw-1"))
        .await;
    assert!(sent.status < 300, "{}", sent.body);
    let id = sent.invocation_id().to_owned();
    let pending = projected(&store, |s| s.pending().is_some())
        .await
        .pending()
        .cloned()
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
        s.events()
            .iter()
            .any(|e| matches!(e, Event::Stalled { .. }))
    })
    .await;
    assert_eq!(s.pending(), Some(&pending), "same draw, same round");
    assert!(
        s.events().iter().any(
            |e| matches!(e, Event::Stalled { reason, round, .. } if reason == "draw cancelled" && *round == pending.round)
        ),
        "{:#?}",
        s.events()
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
    let state: Drawing = serde_json::from_value(reply.body).unwrap();
    assert_eq!(state.pending(), Some(&pending));
}
