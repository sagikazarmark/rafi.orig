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
use restate_e2e_harness::{Call, ReusePolicy, ServerSpec, launcher_or_skip};
use restate_rafi::{Raffle, RecordingProjector, SERVICE_NAME};
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
async fn projected(projector: &RecordingProjector, f: impl Fn(&DrawState) -> bool) -> DrawState {
    for _ in 0..200 {
        if let Some(s) = projector.latest(RAFFLE)
            && f(&s)
        {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "projection not reached; latest: {:#?}",
        projector.latest(RAFFLE)
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
    });
    let projector = RecordingProjector::default();
    restate
        .deploy(
            Endpoint::builder()
                .bind(Raffle::new(drand, projector.clone()))
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
    let frozen = projected(&projector, |s| s.events.is_empty()).await;
    assert_eq!(frozen.entrants.len(), 5);
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(doc)), None)
        .await;
    assert_eq!(reply.status, 200, "refreezing the same list is idempotent");
    let reply = restate
        .invoke(&call("freeze"), Some(&json!(self::doc(4))), None)
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
    let s = projected(&projector, |s| {
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

    // The draw journals the clock, the projections and the beacon under stable names.
    let runs = restate.admin().runs(sent.invocation_id()).await;
    assert_eq!(runs, ["SystemTime::now()", "project", "beacon", "project"]);

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
    let s = projected(&projector, |s| {
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
    let finished = projected(&projector, |s| s.finished).await;
    let reply = restate.invoke(&call("get"), None, None).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.body, Value::Null, "state is cleared after finishing");

    // The projected log verifies independently.
    let report = verify(&doc.canonical_json(), &finished.list_hash, &finished.events);
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), 2);
}
