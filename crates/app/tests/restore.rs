//! The restore in the runbook (`docs/operations/runbook.md`, "Backups and
//! restore"), rehearsed on SQLite files: a backup taken partway through an
//! installation's life, the live database carried on past it, then the
//! runbook's steps in order, with the SQL the runbook and the Cloudflare
//! page give (read from them, so the documents can't drift from what ran).
//! D1 is SQLite: the Workers variant is the same statements on a copy.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rafi_api::{
    AddRequest, AddRow, EntrantPatch, LifecycleCommand, Origin, PrizeInput, SkipReason,
};
use rafi_app::{self as app, App, Clock, EventDate, Limiter, Policy, ProofOfWork, RetentionPolicy};
use rafi_engine::drand::{DrandClient, DrandConfig};
use rafi_engine::local::LocalEngine;
use rafi_lifecycle::{RaffleState, prize::Delivery};
use rafi_live_server::{LiveUpdates, Polling};
use rafi_raffle::Decision;
use rafi_store::{Db, RaffleWriter, raffles};

const RUNBOOK: &str = include_str!("../../../docs/operations/runbook.md");
const CLOUDFLARE: &str = include_str!("../../../docs/operations/cloudflare.md");

const OWNER: app::Actor<'static> = app::Actor::solo("organizer");
/// An organizer who removes their data after the backup.
const LEAVER: app::Actor<'static> = app::Actor::solo("leaver");

/// The SQL of each `--command "…"` in `doc`'s section under `heading`, in
/// order.
fn commands(doc: &str, heading: &str) -> Vec<String> {
    let start = doc.find(heading).unwrap_or_else(|| panic!("no {heading}"));
    let section = &doc[start + heading.len()..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    let mut out = Vec::new();
    let mut rest = section;
    while let Some(at) = rest.find("--command \"") {
        rest = &rest[at + "--command \"".len()..];
        let end = rest.find('"').expect("the command's closing quote");
        out.push(rest[..end].to_owned());
        rest = &rest[end + 1..];
    }
    out
}

/// `sql` split into its statements, without those that are only comments.
/// A statement in the documents ends its line with its semicolon; one in a
/// string literal (a statement the query writes) never does.
fn statements(sql: &str) -> Vec<String> {
    sql.split(";\n")
        .filter(|s| {
            s.lines()
                .any(|l| !l.trim().is_empty() && !l.trim().starts_with("--"))
        })
        .map(str::to_owned)
        .collect()
}

fn url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

/// The app on the SQLite file at `path`, by `clock`; with the in-process
/// engine if `engine`.
async fn app_on(path: &Path, clock: &Clock, engine: bool) -> App {
    let db = Db::connect(&url(path)).await.expect("database");
    let drand = DrandClient::new(DrandConfig {
        relays: vec![],
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
    })
    .expect("drand");
    let chain = drand.chain_hash();
    let engine = engine.then(|| {
        Arc::new(LocalEngine::new(
            drand,
            RaffleWriter::with_clock(db.clone(), clock.clone()),
            Polling,
        )) as Arc<dyn rafi_engine::Engine>
    });
    App {
        db,
        chain,
        engine,
        live_updates: LiveUpdates::polling(),
        limiter: Arc::new(Limiter::default()),
        clock: clock.clone(),
        policy: Policy {
            public_url: "http://localhost".into(),
            max_entrants: 100,
            proof_of_work: ProofOfWork::new("test-key", 1).unwrap(),
            retention: RetentionPolicy {
                days: 7,
                max_days: 30,
                winner_hold_days: 90,
            },
            min_age: 16,
        },
    }
}

/// A list of `rows`: a name, an email and a registration ID each.
fn list(rows: &[(&str, Option<&str>, Option<&str>)]) -> AddRequest {
    AddRequest {
        batch_id: app::ids::id(),
        origin: Origin::List,
        rows: rows
            .iter()
            .map(|(name, email, list_ref)| AddRow {
                name: (*name).into(),
                email: email.map(Into::into),
                list_ref: list_ref.map(Into::into),
                ..AddRow::default()
            })
            .collect(),
        name_format: None,
    }
}

/// An open raffle of `actor`'s with one prize, its people kept for
/// `retention_days`, and `rows` added; returns its ID and theirs, in order.
async fn open(
    app: &App,
    actor: app::Actor<'_>,
    title: &str,
    retention_days: i64,
    rows: &[(&str, Option<&str>, Option<&str>)],
) -> (String, Vec<String>) {
    let id = app::create(app, actor, title, &EventDate::today(app))
        .await
        .unwrap();
    let mut settings = app::settings_of(&raffles::get(&app.db, &id).await.unwrap().unwrap());
    settings.organizer_name = format!("{title} organizers");
    settings.contact_email = "hello@example.com".into();
    settings.retention_days = retention_days;
    app::update_settings(app, actor, &id, settings)
        .await
        .unwrap();
    app::upsert_prize(
        app,
        actor,
        &id,
        PrizeInput {
            id: None,
            name: "Book".into(),
            sponsor: String::new(),
            description: String::new(),
            qty: 1,
            position: 0,
            delivery: Delivery::Later,
        },
    )
    .await
    .unwrap();
    for step in [LifecycleCommand::Publish, LifecycleCommand::Open] {
        app::lifecycle(app, actor, &id, step).await.unwrap();
    }
    let added = app::add_entrants(app, actor, &id, list(rows))
        .await
        .unwrap();
    assert_eq!(added.added.len(), rows.len(), "{added:?}");
    let ids = added.added.into_iter().map(|a| a.id).collect();
    (id, ids)
}

/// Freezes raffle `id` and, if `finish`, finishes it without a draw.
async fn freeze(app: &App, actor: app::Actor<'_>, id: &str, finish: bool) {
    app::start_draw(app, actor, id).await.unwrap();
    if !finish {
        return;
    }
    app::command(app, actor, id, Decision::Finish, &format!("finish-{id}"))
        .await
        .unwrap();
    for _ in 0..200 {
        if raffles::get(&app.db, id).await.unwrap().unwrap().state == RaffleState::Finished {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("never finished");
}

type Entrant = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
);
type Raffle = (
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
);
type Removal = (String, Option<String>, Option<String>, i64);

/// What the restore is about, table by table.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    raffles: Vec<Raffle>,
    entrants: Vec<Entrant>,
    removals: Vec<Removal>,
    defaults: Vec<(String, String)>,
    erasures: Vec<(String, String)>,
}

impl Snapshot {
    async fn of(db: &Db) -> Self {
        Self {
            raffles: db
                .query(
                    "SELECT id, state, organizer_name, contact_email, privacy_email, purged_at,
                            organizer_left_at, manifest_hash
                     FROM raffles ORDER BY id",
                )
                .all()
                .await
                .unwrap(),
            entrants: db
                .query(
                    "SELECT id, raffle_id, display_name, email_raw, email_norm, list_ref, token_hash,
                            claim_code, erased, hidden, objected_at
                     FROM entrants ORDER BY id",
                )
                .all()
                .await
                .unwrap(),
            removals: db
                .query(
                    "SELECT raffle_id, email_norm, list_ref, removed_at FROM add_removals
                     ORDER BY raffle_id, removed_at, email_norm, list_ref",
                )
                .all()
                .await
                .unwrap(),
            defaults: db
                .query("SELECT owner_id, organizer_name FROM owner_defaults ORDER BY owner_id")
                .all()
                .await
                .unwrap(),
            erasures: db
                .query(
                    "SELECT DISTINCT raffle_id, entrant_id FROM erasures ORDER BY raffle_id, entrant_id",
                )
                .all()
                .await
                .unwrap(),
        }
    }

    /// What of `self` (the live database) a restore of `backup` should
    /// leave: everything that was there at the backup, as it is now, and
    /// nothing the backup didn't have.
    fn as_restored_from(&self, backup: &Snapshot) -> Self {
        let raffle_ids: Vec<&String> = backup.raffles.iter().map(|r| &r.0).collect();
        let entrant_ids: Vec<&String> = backup.entrants.iter().map(|e| &e.0).collect();
        let owners: Vec<&String> = backup.defaults.iter().map(|d| &d.0).collect();
        Self {
            raffles: self
                .raffles
                .iter()
                .filter(|r| raffle_ids.contains(&&r.0))
                .cloned()
                .collect(),
            entrants: self
                .entrants
                .iter()
                .filter(|e| entrant_ids.contains(&&e.0))
                .cloned()
                .collect(),
            removals: self
                .removals
                .iter()
                .filter(|r| raffle_ids.contains(&&r.0))
                .cloned()
                .collect(),
            defaults: self
                .defaults
                .iter()
                .filter(|d| owners.contains(&&d.0))
                .cloned()
                .collect(),
            erasures: self
                .erasures
                .iter()
                .filter(|e| raffle_ids.contains(&&e.0))
                .cloned()
                .collect(),
        }
    }
}

/// Runs each of `sql` on `db`.
async fn run_all(db: &Db, sql: &[String]) {
    for statement in sql {
        db.query(statement.as_str())
            .run()
            .await
            .unwrap_or_else(|e| panic!("{e}: {statement}"));
    }
}

/// A fresh, empty directory for the test's files.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rafi-restore-{}", app::ids::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The runbook's restore, in its order, against a backup taken before an
/// organizer erased someone early, purged a raffle, deleted another, left
/// (one raffle purged, one deleted, their defaults gone); before someone
/// was removed at their request and someone without asking, someone
/// added after the freeze took themselves off, a name was hidden, and a
/// raffle expired. After the restore, the runbook's statements and the
/// re-applied erasures (natively `reapply_erasures`, on D1 the Cloudflare
/// page's SQL), nobody's personal data is back, and everything the backup
/// had is as it is live; a later add still leaves out whom it should. Then
/// the runbook's request-for-data and pre-deploy queries run on the result.
#[tokio::test]
async fn a_restore_puts_nobody_back() {
    let dir = scratch();
    let clock = Clock::fixed(Clock::system().now());
    let live = app_on(&dir.join("live.db"), &clock, true).await;

    // Before the backup.
    let anna = ("Anna", Some("Anna.K+meetup@GoogleMail.com"), None);
    let (meetup, m) = open(
        &live,
        OWNER,
        "Meetup",
        7,
        &[
            anna,
            ("Bence", Some("bence@x.org"), Some("7")),
            ("Cili", Some("cili@x.org"), None),
        ],
    )
    .await;
    let (frozen, f) = open(
        &live,
        OWNER,
        "Frozen",
        7,
        &[
            anna,
            ("Emil", Some("emil@x.org"), None),
            ("Fanni", None, Some("f1")),
        ],
    )
    .await;
    freeze(&live, OWNER, &frozen, false).await;
    let (early, e) = open(
        &live,
        OWNER,
        "Early",
        7,
        &[("Ede", Some("ede@x.org"), None), ("Eva", None, None)],
    )
    .await;
    freeze(&live, OWNER, &early, true).await;
    let (purged, _) = open(
        &live,
        OWNER,
        "Purged",
        7,
        &[("Pal", Some("pal@x.org"), None)],
    )
    .await;
    freeze(&live, OWNER, &purged, true).await;
    let (deleted, _) = open(
        &live,
        OWNER,
        "Deleted",
        7,
        &[("Dora", Some("dora@x.org"), None)],
    )
    .await;
    let (expiring, _) = open(
        &live,
        OWNER,
        "Expiring",
        1,
        &[("Gabi", Some("gabi@x.org"), None)],
    )
    .await;
    let (untouched, _) = open(
        &live,
        OWNER,
        "Untouched",
        7,
        &[
            ("Ulla", Some("ulla@x.org"), Some("u1")),
            ("Uwe", None, None),
        ],
    )
    .await;
    let (left_over, _) = open(
        &live,
        LEAVER,
        "Left over",
        7,
        &[("Lili", Some("lili@x.org"), None)],
    )
    .await;
    freeze(&live, LEAVER, &left_over, true).await;
    let (left_open, _) = open(
        &live,
        LEAVER,
        "Left open",
        7,
        &[("Lea", Some("lea@x.org"), None)],
    )
    .await;
    app::save_defaults(
        &live,
        LEAVER,
        rafi_api::RaffleDefaults {
            organizer_name: "Leaving Ltd".into(),
            contact_email: "leaving@example.com".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app::save_defaults(
        &live,
        OWNER,
        rafi_api::RaffleDefaults {
            organizer_name: "Staying Ltd".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // The backup.
    let backup = dir.join("backup.db");
    live.db
        .query(format!("VACUUM INTO '{}'", backup.display()))
        .run()
        .await
        .unwrap();

    // After it.
    clock.advance(60);
    app::remove_entrant(&live, OWNER, &meetup, &m[1], true)
        .await
        .unwrap();
    app::remove_entrant(&live, OWNER, &meetup, &m[2], false)
        .await
        .unwrap();
    app::add_entrants(
        &live,
        OWNER,
        &meetup,
        list(&[("Dani", Some("dani@x.org"), None)]),
    )
    .await
    .unwrap();
    app::object_entrant(&live, OWNER, &frozen, &f[1])
        .await
        .unwrap();
    let hide = EntrantPatch {
        hidden: Some(true),
        ..Default::default()
    };
    app::patch_entrant(&live, OWNER, &frozen, &f[2], hide)
        .await
        .unwrap();
    app::remove_entrant(&live, OWNER, &early, &e[0], false)
        .await
        .unwrap();
    app::purge(&live, OWNER, &purged).await.unwrap();
    app::delete(&live, OWNER, &deleted).await.unwrap();
    app::remove_owner_data(&live, LEAVER).await.unwrap();
    clock.advance(3 * 86_400);
    let report = app::housekeeping::run(&live).await.unwrap();
    assert_eq!(report.expired, 1, "{report:?}");

    let now = Snapshot::of(&live.db).await;
    let then = Snapshot::of(&Db::connect(&url(&backup)).await.unwrap()).await;
    // What the restore would undo is there to undo.
    for gone in [&m[1], &m[2]] {
        assert!(then.entrants.iter().any(|e| &e.0 == gone));
        assert!(!now.entrants.iter().any(|e| &e.0 == gone));
    }
    for gone in [&deleted, &left_open] {
        assert!(then.raffles.iter().any(|r| &r.0 == gone));
        assert!(!now.raffles.iter().any(|r| &r.0 == gone));
    }
    assert!(now.removals.iter().any(|r| r.0 == meetup) && then.removals.is_empty());
    assert!(then.defaults.iter().any(|d| d.0 == LEAVER.owner));
    assert!(
        then.entrants.iter().all(|e| e.8 == 0),
        "nobody was erased at the backup"
    );
    let expected = now.as_restored_from(&then);
    assert!(expected.entrants.iter().any(|e| e.8 == 1));

    // 1. Before restoring, the statements that put back what it undoes.
    let [save, drawing] = &commands(RUNBOOK, "## Backups and restore")[..] else {
        panic!("the query saving what the restore undoes, and step 5's");
    };
    let mut put_back = Vec::new();
    for query in statements(save) {
        let rows: Vec<(String,)> = live.db.query(query).all().await.unwrap();
        put_back.extend(rows.into_iter().map(|(sql,)| sql));
    }
    let reapply_d1 = statements(&commands(CLOUDFLARE, "## Data residency")[0]);

    for d1 in [false, true] {
        // 2. The restore.
        let path = dir.join(format!("restored-{d1}.db"));
        std::fs::copy(&backup, &path).unwrap();
        let restored = app_on(&path, &clock, false).await;
        for round in ["once", "again"] {
            // 3. What step 1 saved, put back.
            run_all(&restored.db, &put_back).await;
            // 4. The erasures re-applied.
            if d1 {
                run_all(&restored.db, &reapply_d1).await;
            } else {
                app::reapply_erasures(&restored).await.unwrap();
            }
            let snapshot = Snapshot::of(&restored.db).await;
            assert_eq!(snapshot.raffles, expected.raffles, "d1={d1}, {round}");
            assert_eq!(snapshot.entrants, expected.entrants, "d1={d1}, {round}");
            assert_eq!(snapshot.removals, expected.removals, "d1={d1}, {round}");
            assert_eq!(snapshot.defaults, expected.defaults, "d1={d1}, {round}");
            assert_eq!(snapshot.erasures, expected.erasures, "d1={d1}, {round}");
        }

        // Removed at his request, Bence stays out of a later add, by his
        // email and by his registration ID.
        let again = app::add_entrants(
            &restored,
            OWNER,
            &meetup,
            list(&[
                ("Bence", Some("Bence@x.org"), None),
                ("B.", None, Some("7")),
            ]),
        )
        .await
        .unwrap();
        assert!(again.added.is_empty(), "{again:?}");
        assert!(
            again
                .skipped
                .iter()
                .all(|s| s.reason == SkipReason::RemovedAtRequest)
        );

        // 5. The raffles being drawn, for their organizers.
        let drawn: Vec<(String,)> = restored
            .db
            .query(format!("SELECT id FROM ({drawing})"))
            .all()
            .await
            .unwrap();
        assert_eq!(drawn, [(frozen.clone(),)]);

        // The runbook's request for data finds Anna in both raffles she is
        // in, by her email key.
        let [dsar] = &commands(RUNBOOK, "## A person asks for their data")[..] else {
            panic!("one query for a person's data");
        };
        let found: Vec<(String, String)> = restored
            .db
            .query(format!(
                "SELECT raffle, display_name FROM ({dsar}) ORDER BY raffle"
            ))
            .all()
            .await
            .unwrap();
        let mut both = vec![meetup.clone(), frozen.clone()];
        both.sort();
        assert_eq!(
            found,
            both.into_iter()
                .map(|r| (r, "Anna".to_owned()))
                .collect::<Vec<_>>()
        );

        // The pre-deploy query lists the raffles that are live.
        let [preflight] = &commands(RUNBOOK, "## Before a deploy")[..] else {
            panic!("one pre-deploy query");
        };
        let live_now: Vec<(String,)> = restored
            .db
            .query(format!("SELECT id FROM ({preflight}) ORDER BY id"))
            .all()
            .await
            .unwrap();
        let mut expected_live = vec![meetup.clone(), frozen.clone(), untouched.clone()];
        expected_live.sort();
        assert_eq!(
            live_now.into_iter().map(|(id,)| id).collect::<Vec<_>>(),
            expected_live
        );
        assert!(!expected_live.contains(&expiring));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
