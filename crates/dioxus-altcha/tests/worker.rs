//! The drift test: worker.js against the `altcha` crate. Challenges issued in
//! Rust, solved by worker.js under node (WebCrypto, as in a browser), checked
//! with `altcha::verify_solution`.

use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use altcha::{Challenge, CreateChallengeOptions, Solution, VerifySolutionOptions};
use serde_json::{Value, json};

const KEY: &str = "drift-test-key";

fn issue(algorithm: &str, cost: u32, key_prefix: &str) -> Challenge {
    altcha::create_challenge(CreateChallengeOptions {
        algorithm: algorithm.into(),
        cost,
        key_prefix: key_prefix.into(),
        hmac_signature_secret: Some(KEY.into()),
        expires_at: Some(4_000_000_000),
        ..Default::default()
    })
    .unwrap()
}

/// Runs `altchaSolve` under node for each `(challenge JSON, start, step)`.
fn node_solve(jobs: &[(Value, u32, u32)]) -> Vec<Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let input: Vec<Value> = jobs
        .iter()
        .map(|(c, start, step)| json!({ "challenge": c, "start": start, "step": step }))
        .collect();
    let mut child = Command::new("node")
        .arg(dir.join("tests/solve.mjs"))
        .arg(dir.join("src/worker.js"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the drift test needs node (18+) on PATH");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&input).unwrap().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "node failed: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn verified(challenge: &Challenge, solution: &Value) -> bool {
    let solution: Solution = serde_json::from_value(solution.clone())
        .unwrap_or_else(|e| panic!("not a solution: {solution} ({e})"));
    altcha::verify_solution(VerifySolutionOptions::new(challenge, &solution, KEY))
        .unwrap()
        .verified
}

#[test]
fn worker_solutions_verify() {
    let cases = [
        ("PBKDF2/SHA-256", 10, "00"),
        ("PBKDF2/SHA-384", 10, "0"),
        ("PBKDF2/SHA-512", 10, "a"),
        // cost 0 runs as 1 round on both sides.
        ("PBKDF2/SHA-256", 0, "0"),
        ("SHA-256", 1, "00"),
        ("SHA-384", 3, "0"),
        ("SHA-512", 1, "f"),
        // Odd-length prefix, compared as a hex string.
        ("PBKDF2/SHA-256", 5, "01a"),
    ];
    let challenges: Vec<Challenge> = cases
        .iter()
        .map(|(alg, cost, prefix)| issue(alg, *cost, prefix))
        .collect();
    let jobs: Vec<_> = challenges
        .iter()
        .map(|c| (serde_json::to_value(c).unwrap(), 0, 1))
        .collect();
    for ((case, challenge), solution) in cases.iter().zip(&challenges).zip(node_solve(&jobs)) {
        assert!(verified(challenge, &solution), "{case:?}: {solution}");
    }
}

#[test]
fn split_counters_verify() {
    // Worker i of n tries i, i + n, ...: any worker's find verifies.
    let challenge = issue("PBKDF2/SHA-256", 5, "0");
    let json = serde_json::to_value(&challenge).unwrap();
    let jobs: Vec<_> = (0..3).map(|i| (json.clone(), i, 3)).collect();
    for (i, solution) in node_solve(&jobs).iter().enumerate() {
        assert_eq!(solution["counter"].as_u64().unwrap() % 3, i as u64);
        assert!(verified(&challenge, solution), "worker {i}: {solution}");
    }
}

#[test]
fn uppercase_prefix_and_lowercase_key() {
    // A prefix sent in uppercase still matches; the key comes back lowercase.
    let challenge = issue("PBKDF2/SHA-256", 5, "0a");
    let mut json = serde_json::to_value(&challenge).unwrap();
    json["parameters"]["keyPrefix"] = "0A".into();
    let solution = &node_solve(&[(json, 0, 1)])[0];
    let key = solution["derivedKey"].as_str().unwrap();
    assert_eq!(key, key.to_lowercase());
    assert!(verified(&challenge, solution), "{solution}");
}

#[test]
fn refuses_what_it_cant_solve() {
    let base = serde_json::to_value(issue("PBKDF2/SHA-256", 1, "0")).unwrap();
    let mut scrypt = base.clone();
    scrypt["parameters"]["algorithm"] = "SCRYPT".into();
    let mut bad_nonce = base.clone();
    bad_nonce["parameters"]["nonce"] = "xyz".into();
    let mut bad_prefix = base;
    bad_prefix["parameters"]["keyPrefix"] = "zz".into();
    let out = node_solve(&[(scrypt, 0, 1), (bad_nonce, 0, 1), (bad_prefix, 0, 1)]);
    for (want, got) in [
        "unsupported algorithm",
        "invalid nonce",
        "invalid keyPrefix",
    ]
    .iter()
    .zip(&out)
    {
        let error = got["error"].as_str().unwrap_or_default();
        assert!(error.contains(want), "{want}: {got}");
    }
}

#[test]
fn worker_js_stays_es2017() {
    // A cheap guard for the syntax old Safari can't parse; a parse failure
    // would only show up as a worker `error` on those phones.
    let source = dioxus_altcha::WORKER_JS;
    for token in ["?.", "??", "class ", "import ", "export ", "static ", "#"] {
        let code = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!code.contains(token), "worker.js uses {token:?}");
    }
}
