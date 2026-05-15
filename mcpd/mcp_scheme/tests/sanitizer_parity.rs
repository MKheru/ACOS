//! WS2.M4 — Parity scorecard for the Rust sanitizer against the corpus
//! produced by the Python `agent.mcp_sanitizer` regression suite.
//!
//! The corpus (`tests/fixtures/attack_corpus_v2.jsonl`) is 95 items:
//! 70 attacks + 25 benign. Each line is a JSON object with `id`,
//! `label` (`"attack"` or `"benign"`), `category`, `text` and (for
//! attacks) `expected_pattern`.
//!
//! Python baseline (`parity_python_v2.json`):
//! ```
//! detection_rate    : 0.9857  (69 / 70 attacks caught)
//! false_positive_rate: 0.40   (10 / 25 benign flagged)
//! precision         : 0.8734
//! recall            : 0.9857
//! f1                : 0.9262
//! missed            : indirect_doc_citation × 1
//! ```
//!
//! This Rust port intentionally implements a *subset* of Patch 16
//! patterns (see `sanitizer.rs` doc-header for the scope statement).
//! Initial commit hit 0.6714 detection. Adding the b64/hex/ROT13/
//! `\uXXXX` substring decoders pushed it to **0.7571** without
//! increasing the false-positive rate.
//!
//! Current bars:
//!   * detection_rate >= 0.70    — ≥ 49 of 70 attacks caught (we are
//!     at 53, 0.7571)
//!   * false_positive_rate <= 0.40 — no worse than Python (we are at
//!     0.28, *better* than Python's 0.40)
//!
//! The test prints a per-category breakdown so the gap list is visible
//! in CI output even when overall thresholds are met. As more patterns
//! land the bars should tighten toward the Python baseline (0.9857 /
//! 0.40).

use std::fs;
use std::path::PathBuf;

use mcp_scheme::sanitizer::Sanitizer;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CorpusItem {
    id: String,
    label: String,
    category: String,
    text: String,
    #[serde(default)]
    expected_pattern: Option<String>,
}

fn load_corpus() -> Vec<CorpusItem> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/attack_corpus_v2.jsonl");
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read corpus fixture at {path:?}: {e}"));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("corpus line {i} is not valid JSON: {e}"))
        })
        .collect()
}

struct ScoreCard {
    attacks: usize,
    benign: usize,
    /// Caught attacks (true positives).
    tp: usize,
    /// Missed attacks (false negatives) — list ids for diagnosis.
    fn_ids: Vec<String>,
    /// Wrongly-flagged benign items (false positives).
    fp_ids: Vec<String>,
    /// Per-attack-category miss counts (helps identify which families
    /// the subset port doesn't yet cover).
    missed_by_category: std::collections::BTreeMap<String, usize>,
}

fn score(items: &[CorpusItem], sanitizer: &Sanitizer) -> ScoreCard {
    let mut sc = ScoreCard {
        attacks: 0,
        benign: 0,
        tp: 0,
        fn_ids: Vec::new(),
        fp_ids: Vec::new(),
        missed_by_category: std::collections::BTreeMap::new(),
    };
    for item in items {
        let detected = sanitizer.check(&item.text).is_some();
        match item.label.as_str() {
            "attack" => {
                sc.attacks += 1;
                if detected {
                    sc.tp += 1;
                } else {
                    sc.fn_ids.push(item.id.clone());
                    *sc.missed_by_category.entry(item.category.clone()).or_insert(0) += 1;
                }
            }
            "benign" => {
                sc.benign += 1;
                if detected {
                    sc.fp_ids.push(item.id.clone());
                }
            }
            other => panic!("unknown label '{other}' in corpus item {}", item.id),
        }
    }
    sc
}

#[test]
fn parity_thresholds_meet_published_target() {
    let items = load_corpus();
    let sanitizer = Sanitizer::new();
    let sc = score(&items, &sanitizer);

    let detection_rate = sc.tp as f64 / sc.attacks as f64;
    let fp_rate = sc.fp_ids.len() as f64 / sc.benign as f64;

    // ------------------------------------------------------------------
    // Diagnostic dump — always print so CI can see the gap list even
    // when the assertions pass. Printed via eprintln (stderr) so cargo
    // test renders it even on success.
    // ------------------------------------------------------------------
    eprintln!("=== Rust sanitizer parity scorecard ===");
    eprintln!("corpus_size       : {}", items.len());
    eprintln!("attacks           : {}", sc.attacks);
    eprintln!("benign            : {}", sc.benign);
    eprintln!("tp (caught)       : {}", sc.tp);
    eprintln!("fn (missed)       : {}", sc.fn_ids.len());
    eprintln!("fp (false flag)   : {}", sc.fp_ids.len());
    eprintln!("detection_rate    : {:.4}", detection_rate);
    eprintln!("fp_rate           : {:.4}", fp_rate);
    if !sc.missed_by_category.is_empty() {
        eprintln!("missed_by_category:");
        for (cat, n) in &sc.missed_by_category {
            eprintln!("  {cat:32}  {n}");
        }
    }
    if !sc.fn_ids.is_empty() && sc.fn_ids.len() <= 30 {
        eprintln!("missed_ids        : {:?}", sc.fn_ids);
    }
    if !sc.fp_ids.is_empty() && sc.fp_ids.len() <= 30 {
        eprintln!("fp_ids            : {:?}", sc.fp_ids);
    }
    eprintln!("=== end scorecard ===");

    // Bars chosen for a partial port (see sanitizer.rs doc-header).
    // If a future commit grows coverage the bars can be tightened
    // toward the Python baseline (0.9857 / 0.40).
    assert!(
        detection_rate >= 0.70,
        "detection_rate {:.4} below 0.70 floor — see scorecard above",
        detection_rate
    );
    assert!(
        fp_rate <= 0.40,
        "fp_rate {:.4} above 0.40 ceiling (Python baseline) — see scorecard above",
        fp_rate
    );
}

#[test]
fn every_corpus_item_parses_and_has_required_fields() {
    let items = load_corpus();
    assert!(!items.is_empty(), "corpus must not be empty");
    for item in &items {
        assert!(!item.id.is_empty(), "id missing in corpus item");
        assert!(!item.text.is_empty(), "text missing in {}", item.id);
        assert!(
            item.label == "attack" || item.label == "benign",
            "{} has unknown label {}",
            item.id,
            item.label
        );
        // `expected_pattern` is optional on attack items in the v2 corpus
        // (some entries describe the attack category implicitly via
        // `category` alone — e.g. `multi_stage_combo`).
    }
}
