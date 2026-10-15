//! `/bench`: the plan's M4 measurements, on whatever device opens the page.
//!
//! `/bench?costs=1000,5000&workers=1,2,4&runs=30` — for each cost and worker
//! count, the median and p95 time to solve (fresh challenges, prefix "00");
//! other challenges at the same cost solved by `altcha`'s Rust in wasm on the
//! main thread (the §2.1 comparison); and one verification's time (wasm, the
//! proxy for a Worker's CPU). A setting that can't run says why (e.g. plain
//! HTTP: WebCrypto needs a secure context). The results end with a line
//! `done`, and are also in `window.__bench` (JSON) for scripts. It issues its
//! own challenges in the page, so it measures the device alone.

use dioxus::prelude::*;

#[component]
pub fn Bench() -> Element {
    let lines = use_signal(Vec::<String>::new);
    #[cfg(feature = "web")]
    use_hook(|| spawn(web::run(lines)));
    rsx! {
        p { class: "lead", "Keep this page in front until it says done." }
        section { class: "card",
            pre { id: "bench", {lines().join("\n")} }
        }
    }
}

#[cfg(feature = "web")]
mod web {
    use std::time::Duration;

    use dioxus::prelude::*;
    use dioxus_altcha::{RawChallenge, SolveOptions, altcha, solve};

    /// The bench's key: challenges never leave the page.
    const KEY: &str = "bench-only-key";

    fn param(search: &str, name: &str) -> Option<String> {
        search
            .trim_start_matches('?')
            .split('&')
            .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=').map(str::to_owned))
    }

    fn list(search: &str, name: &str, default: &[u32]) -> Vec<u32> {
        param(search, name)
            .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_else(|| default.to_vec())
    }

    fn now() -> f64 {
        js_sys::Date::now()
    }

    /// The median and p95, or `None` with no samples.
    fn stats(mut ms: Vec<f64>) -> Option<(f64, f64)> {
        if ms.is_empty() {
            return None;
        }
        ms.sort_by(f64::total_cmp);
        let at = |q: f64| ms[((ms.len() as f64 - 1.0) * q).round() as usize];
        Some((at(0.5), at(0.95)))
    }

    /// `median=…ms p95=…ms`, or why there's nothing to report.
    fn summary(ms: Vec<f64>, digits: usize, failure: &Option<String>) -> String {
        match stats(ms) {
            Some((median, p95)) => format!("median={median:.digits$}ms p95={p95:.digits$}ms"),
            None => format!("no result: {}", failure.as_deref().unwrap_or("nothing ran")),
        }
    }

    fn challenge(cost: u32) -> altcha::Challenge {
        altcha::create_challenge(altcha::CreateChallengeOptions {
            algorithm: "PBKDF2/SHA-256".into(),
            cost,
            key_prefix: "00".into(),
            expires_at: Some((now() / 1000.0) as i64 + 600),
            hmac_signature_secret: Some(KEY.into()),
            ..Default::default()
        })
        .expect("valid options")
    }

    async fn yield_now() {
        // Lets the page paint between runs.
        let _ = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
            let _ = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        }))
        .await;
    }

    pub async fn run(mut lines: Signal<Vec<String>>) {
        {
            {
                let search = web_sys::window()
                    .unwrap()
                    .location()
                    .search()
                    .unwrap_or_default();
                let costs = list(&search, "costs", &[1000, 5000]);
                let workers = list(&search, "workers", &[1, 2, 4]);
                let runs = list(&search, "runs", &[30])[0].max(2) as usize;
                let mut out = Vec::new();
                let mut log = move |line: String, out: &mut Vec<String>| {
                    out.push(line.clone());
                    lines.write().push(line);
                };
                log(
                    format!("runs={runs} prefix=00 hardwareConcurrency={}", hardware()),
                    &mut out,
                );
                for &cost in &costs {
                    for &n in &workers {
                        let (mut ms, mut failure) = (Vec::new(), None);
                        for _ in 0..runs {
                            let raw = RawChallenge::from_challenge(&challenge(cost));
                            let opts = SolveOptions::new()
                                .workers(n as usize)
                                .timeout(Some(Duration::from_secs(300)));
                            let t = now();
                            match solve(&raw, &opts).await {
                                Ok(_) => ms.push(now() - t),
                                // E.g. plain HTTP: WebCrypto needs a secure context.
                                Err(e) => failure = Some(e.to_string()),
                            }
                            yield_now().await;
                        }
                        log(
                            format!("solve cost={cost} workers={n} {}", summary(ms, 0, &failure)),
                            &mut out,
                        );
                    }
                    // Other challenges at the same cost, in altcha's Rust, wasm, one thread
                    // (fewer runs: it blocks the page).
                    let (mut ms, mut verify, mut failure) = (Vec::new(), Vec::new(), None);
                    for _ in 0..runs.min(10) {
                        let c = challenge(cost);
                        let t = now();
                        match altcha::solve_challenge(altcha::SolveChallengeOptions::new(&c)) {
                            Ok(Some(s)) => {
                                ms.push(now() - t);
                                let t = now();
                                let _ = altcha::verify_solution(
                                    altcha::VerifySolutionOptions::new(&c, &s, KEY),
                                );
                                verify.push(now() - t);
                            }
                            Ok(None) => failure = Some("timed out".into()),
                            Err(e) => failure = Some(e.to_string()),
                        }
                        yield_now().await;
                    }
                    log(
                        format!(
                            "rust-wasm cost={cost} workers=1 {}",
                            summary(ms, 0, &failure)
                        ),
                        &mut out,
                    );
                    log(
                        format!("verify cost={cost} {}", summary(verify, 1, &failure)),
                        &mut out,
                    );
                }
                log("done".into(), &mut out);
                let _ = js_sys::Reflect::set(
                    &js_sys::global(),
                    &"__bench".into(),
                    &serde_json_lines(&out).into(),
                );
            }
        }
    }

    fn hardware() -> String {
        js_sys::Reflect::get(&js_sys::global(), &"navigator".into())
            .and_then(|n| js_sys::Reflect::get(&n, &"hardwareConcurrency".into()))
            .ok()
            .and_then(|v| v.as_f64())
            .map_or("unknown".into(), |n| n.to_string())
    }

    fn serde_json_lines(lines: &[String]) -> String {
        let quoted: Vec<String> = lines.iter().map(|l| format!("{l:?}")).collect();
        format!("[{}]", quoted.join(","))
    }
}
