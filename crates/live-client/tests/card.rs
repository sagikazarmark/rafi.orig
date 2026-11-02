//! Server-renders [`Card`] for every [`Screen`] variant.

use dioxus::prelude::*;
use rafi_live_client::{
    CARD, Card, CardStrings, Current, DrawInput, DrawOutcome, ParticipantView, Phase, Screen, WON,
    WonPrize, screen,
};

const T: u64 = 1_700_000_000;

/// Each string as its field name in brackets, to tell which one shows.
fn strings() -> CardStrings {
    let t = |k: &str| format!("[{k}]");
    CardStrings {
        you_won: t("you_won"),
        go_to_organizer: t("go_to_organizer"),
        thanks_won: t("thanks_won"),
        sent_later: t("sent_later"),
        thanks: t("thanks"),
        drawing: t("drawing"),
        not_this_time: t("not_this_time"),
        draw_starting: t("draw_starting"),
        prizes_left: t("prizes_left"),
        waiting: t("waiting"),
        locking: t("locking"),
        drand_round: t("drand_round"),
    }
}

fn draw(id: &str, prize: &str, outcome: DrawOutcome) -> DrawInput {
    DrawInput {
        entrant_id: id.into(),
        display_name: format!("Name {id}"),
        prize_name: prize.into(),
        outcome,
        sent_later: false,
        no_phone: false,
    }
}

/// [`Screen::Won`] with `(prize_name, claimed, sent_later)` each.
fn won(prizes: &[(&str, bool, bool)], done: bool) -> Screen {
    Screen::Won {
        prizes: prizes
            .iter()
            .map(|&(name, claimed, sent_later)| WonPrize {
                prize_name: name.into(),
                claimed,
                sent_later,
            })
            .collect(),
        done,
    }
}

fn mug() -> Option<Current> {
    Some(Current {
        prize_name: "Mug".into(),
        round: 42,
        round_time: T,
    })
}

struct Case {
    view: ParticipantView,
    me: Option<&'static str>,
    page_state: Phase,
    now_ms: u64,
    announce: bool,
}

impl Case {
    fn new(state: Phase, current: Option<Current>, draws: Vec<DrawInput>) -> Self {
        Self {
            view: ParticipantView::new(1, state, current, draws, 2),
            me: None,
            page_state: state,
            now_ms: T * 1000 - 3000,
            announce: false,
        }
    }

    fn me(mut self, me: &'static str) -> Self {
        self.me = Some(me);
        self
    }

    fn screen(&self) -> Screen {
        screen(&self.view, self.me, self.page_state, self.now_ms)
    }

    fn render(&self) -> String {
        let view = self.view.clone();
        let me = self.me.map(String::from);
        let claim_code = self.me.map(|_| "AB12".to_owned());
        let (page_state, now_ms, announce) = (self.page_state, self.now_ms, self.announce);
        dioxus_ssr::render_element(rsx! {
            Card { view, me, claim_code, page_state, now_ms, strings: strings(), announce }
        })
    }
}

/// Asserts `html` is exactly one `<div class="{class}">…</div>` element.
fn single_root(html: &str, class: &str) {
    let open = format!("<div class=\"{class}\"");
    assert!(html.starts_with(&open), "{html}");
    assert!(html.ends_with("</div>"), "{html}");
    // The first time the depth returns to 0 must be the very end.
    let mut depth = 0i32;
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let end = rest.find('>').expect("tags close");
        let tag = &rest[..=end];
        if tag.starts_with("</") {
            depth -= 1;
            if depth == 0 {
                assert_eq!(rest.len(), tag.len(), "more than one root: {html}");
            }
        } else if !tag.starts_with("<!--") && !tag.ends_with("/>") {
            depth += 1;
        }
        rest = &rest[end + 1..];
    }
    assert_eq!(depth, 0, "{html}");
}

#[test]
fn blank_renders_nothing() {
    let c = Case::new(Phase::Open, None, vec![]);
    assert_eq!(c.screen(), Screen::Blank);
    assert_eq!(c.render(), "");
}

#[test]
fn reload_renders_nothing() {
    let mut c = Case::new(Phase::Open, None, vec![]);
    c.page_state = Phase::Ready;
    assert_eq!(c.screen(), Screen::Reload);
    assert_eq!(c.render(), "");
}

#[test]
fn waiting() {
    let c = Case::new(Phase::Closed, None, vec![]).me("a");
    assert_eq!(c.screen(), Screen::Waiting);
    let html = c.render();
    single_root(&html, CARD);
    assert!(html.contains("animate-float"), "{html}");
    assert!(html.contains("[waiting]"), "{html}");
}

#[test]
fn countdown() {
    let mut c = Case::new(Phase::Drawing, mug(), vec![]).me("a");
    assert!(matches!(c.screen(), Screen::Countdown { secs_left: 3, .. }));
    let html = c.render();
    single_root(&html, CARD);
    for s in [
        "badge badge-primary badge-soft font-semibold",
        "🎲 [drawing]",
        ">Mug<",
        "text-primary\">3</div>",
        "[locking] · [drand_round] 42",
    ] {
        assert!(html.contains(s), "{s} in {html}");
    }

    c.now_ms = T * 1000;
    let html = c.render();
    single_root(&html, CARD);
    assert!(html.contains("text-primary\">…</div>"), "{html}");
}

#[test]
fn won_live() {
    let mut c = Case::new(
        Phase::Drawing,
        None,
        vec![draw("a", "Book", DrawOutcome::Drawn)],
    )
    .me("a");
    c.announce = true;
    assert_eq!(c.screen(), won(&[("Book", false, false)], false));
    let html = c.render();
    single_root(&html, &format!("{WON} animate-pop"));
    for s in [
        "role=\"alert\"",
        "🏆",
        "[you_won]",
        ">Book<",
        "claim-code",
        ">AB12<",
        "[go_to_organizer]",
    ] {
        assert!(html.contains(s), "{s} in {html}");
    }
}

#[test]
fn won_on_page_load_is_quiet() {
    let c = Case::new(
        Phase::Drawing,
        None,
        vec![draw("a", "Book", DrawOutcome::Drawn)],
    )
    .me("a");
    let html = c.render();
    single_root(&html, WON);
    assert!(!html.contains("role="), "{html}");
    assert!(!html.contains("animate-pop"), "{html}");
}

#[test]
fn won_claimed_or_done_says_thanks() {
    let claimed = Case::new(
        Phase::Drawing,
        None,
        vec![draw("a", "Book", DrawOutcome::Claimed)],
    )
    .me("a");
    assert_eq!(claimed.screen(), won(&[("Book", true, false)], false));
    let done = Case::new(
        Phase::Finished,
        None,
        vec![draw("a", "Book", DrawOutcome::Drawn)],
    )
    .me("a");
    assert_eq!(done.screen(), won(&[("Book", false, false)], true));
    for c in [claimed, done] {
        let html = c.render();
        single_root(&html, WON);
        assert!(html.contains("[thanks_won]"), "{html}");
        assert!(!html.contains("[go_to_organizer]"), "{html}");
        assert!(!html.contains("[sent_later]"), "{html}");
    }
}

#[test]
fn won_a_prize_sent_later() {
    let later = |outcome| DrawInput {
        sent_later: true,
        ..draw("a", "Voucher", outcome)
    };
    let drawn = Case::new(Phase::Drawing, None, vec![later(DrawOutcome::Drawn)]).me("a");
    let html = drawn.render();
    assert!(
        html.contains("[go_to_organizer]"),
        "it is claimed with the code all the same: {html}"
    );
    for state in [Phase::Drawing, Phase::Finished] {
        let claimed = Case::new(state, None, vec![later(DrawOutcome::Claimed)]).me("a");
        assert_eq!(
            claimed.screen(),
            won(&[("Voucher", true, true)], state.is_over())
        );
        let html = claimed.render();
        single_root(&html, WON);
        assert!(html.contains("[sent_later]"), "{state:?}: {html}");
        assert!(!html.contains("[thanks_won]"), "{state:?}: {html}");
    }
}

/// One prize: the prize, the code and the note, and no list.
#[test]
fn won_one_prize_has_no_list() {
    let c = Case::new(
        Phase::Drawing,
        None,
        vec![draw("a", "Book", DrawOutcome::Claimed)],
    )
    .me("a");
    let html = c.render();
    single_root(&html, WON);
    assert!(!html.contains("<ul"), "{html}");
    assert_eq!(html.matches("[you_won]").count(), 1, "{html}");
    assert_eq!(html.matches(">AB12<").count(), 1, "{html}");
}

/// More prizes than people: pass 2 drew "a" again. "YOU WON!" and the
/// claim code once; each prize with its own note, in the order drawn.
#[test]
fn won_two_prizes() {
    let mut c = Case::new(
        Phase::Drawing,
        None,
        vec![
            draw("a", "Book", DrawOutcome::Claimed),
            draw("b", "Mug", DrawOutcome::Claimed),
            DrawInput {
                sent_later: true,
                ..draw("a", "Voucher", DrawOutcome::Drawn)
            },
        ],
    )
    .me("a");
    c.announce = true;
    assert_eq!(
        c.screen(),
        won(&[("Book", true, false), ("Voucher", false, true)], false)
    );
    let html = c.render();
    single_root(&html, &format!("{WON} animate-pop"));
    assert_eq!(html.matches("role=\"alert\"").count(), 1, "{html}");
    assert_eq!(html.matches("[you_won]").count(), 1, "{html}");
    assert_eq!(html.matches("claim-code").count(), 1, "{html}");
    assert_eq!(html.matches(">AB12<").count(), 1, "{html}");
    assert_eq!(html.matches("<li").count(), 2, "{html}");
    assert!(!html.contains("Mug"), "someone else's prize: {html}");
    let book = html.find(">Book<").expect("the book");
    let thanks = html.find("[thanks_won]").expect("the book is claimed");
    let voucher = html.find(">Voucher<").expect("the voucher");
    let go = html
        .find("[go_to_organizer]")
        .expect("the voucher is to claim");
    let code = html.find(">AB12<").unwrap();
    assert!(
        book < thanks && thanks < voucher && voucher < go && go < code,
        "each prize with its note, then the code: {html}"
    );

    // Both claimed, the voucher sent later; then the raffle is over.
    for state in [Phase::Drawing, Phase::Finished] {
        let c = Case::new(
            state,
            None,
            vec![
                draw("a", "Book", DrawOutcome::Claimed),
                draw("b", "Mug", DrawOutcome::Claimed),
                DrawInput {
                    sent_later: true,
                    ..draw("a", "Voucher", DrawOutcome::Claimed)
                },
            ],
        )
        .me("a");
        let html = c.render();
        single_root(&html, WON);
        assert!(!html.contains("role="), "{html}");
        assert_eq!(html.matches("[thanks_won]").count(), 1, "{state:?}: {html}");
        assert_eq!(html.matches("[sent_later]").count(), 1, "{state:?}: {html}");
        assert!(!html.contains("[go_to_organizer]"), "{state:?}: {html}");
    }
}

#[test]
fn thanks() {
    let c = Case::new(
        Phase::Finished,
        None,
        vec![draw("a", "Book", DrawOutcome::Claimed)],
    )
    .me("x");
    assert_eq!(c.screen(), Screen::Thanks);
    let html = c.render();
    single_root(&html, CARD);
    assert!(html.contains("🎉"), "{html}");
    assert!(html.contains("[thanks]"), "{html}");
}

#[test]
fn between_draws_before_the_first() {
    let c = Case::new(Phase::Frozen, None, vec![]).me("x");
    assert!(matches!(
        c.screen(),
        Screen::BetweenDraws { last: None, .. }
    ));
    let html = c.render();
    single_root(&html, CARD);
    for s in [
        "🥁",
        "[draw_starting]",
        "2 [prizes_left]",
        "badge-secondary",
    ] {
        assert!(html.contains(s), "{s} in {html}");
    }
}

#[test]
fn between_draws_for_an_entrant_and_a_visitor() {
    let draws = || {
        vec![
            draw("a", "Book", DrawOutcome::Drawn),
            draw("c", "Mug", DrawOutcome::NotHere),
        ]
    };
    let entrant = Case::new(Phase::Drawing, None, draws()).me("x");
    let html = entrant.render();
    single_root(&html, CARD);
    assert!(html.contains("🎉 <bdi>Name a</bdi> → Book"), "{html}");
    assert!(
        !html.contains("Name c"),
        "the no-show isn't celebrated: {html}"
    );
    assert!(html.contains("[not_this_time] 🤞"), "{html}");
    assert!(html.contains("2 [prizes_left]"), "{html}");

    let visitor = Case::new(Phase::Drawing, None, draws());
    assert!(matches!(
        visitor.screen(),
        Screen::BetweenDraws { entered: false, .. }
    ));
    let html = visitor.render();
    single_root(&html, CARD);
    assert!(html.contains("🎉 <bdi>Name a</bdi> → Book"), "{html}");
    assert!(html.contains(">[drawing]<"), "{html}");
    assert!(!html.contains("[not_this_time]"), "{html}");
}

#[test]
fn rendering_is_deterministic() {
    let c = Case::new(Phase::Drawing, mug(), vec![]).me("a");
    assert_eq!(c.render(), c.render());
}
