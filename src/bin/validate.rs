//! Ground-truth validation harness.
//!
//! Usage:
//!   cargo run --bin validate [eval_dir]
//!
//! Reads `<eval_dir>/ground_truth.csv`, runs `analyze_wav` on every file in
//! `<eval_dir>/recordings/`, and reports per-file hits, misses, and false
//! positives, plus an overall sensitivity and FP count.
//!
//! `eval_dir` defaults to `eval/` (relative to the working directory).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use bat_detector::api::{analyze_wav, AnalysisParams};

// ── Ground-truth record ───────────────────────────────────────────────────────

#[derive(Debug)]
struct Annotation {
    filename: String,
    expected_code: String,
    start_s: Option<f32>,
    end_s: Option<f32>,
    notes: String,
}

// ── CSV reader ────────────────────────────────────────────────────────────────

fn read_ground_truth(path: &Path) -> Vec<Annotation> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Cannot read {}: {}", path.display(), e));

    let mut annotations = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Skip header row (starts with "filename")
        if line.starts_with("filename") {
            continue;
        }
        // Split into at most 5 fields (notes may contain commas)
        let cols: Vec<&str> = line.splitn(5, ',').collect();
        if cols.len() < 2 {
            continue;
        }
        annotations.push(Annotation {
            filename:      cols[0].trim().to_string(),
            expected_code: cols[1].trim().to_string(),
            start_s:       cols.get(2).and_then(|s| s.trim().parse::<f32>().ok()),
            end_s:         cols.get(3).and_then(|s| s.trim().parse::<f32>().ok()),
            notes:         cols.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
        });
    }
    annotations
}

// ── Pass overlap helper ───────────────────────────────────────────────────────

/// Returns true if the pass time window overlaps [ann_start, ann_end].
/// If either bound is None the check is unconstrained on that side.
fn overlaps(pass_start: f32, pass_end: f32, ann_start: Option<f32>, ann_end: Option<f32>) -> bool {
    let lo = ann_start.unwrap_or(f32::NEG_INFINITY);
    let hi = ann_end.unwrap_or(f32::INFINITY);
    pass_start <= hi && pass_end >= lo
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let eval_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "eval".to_string());
    let eval_path = Path::new(&eval_dir);
    let recordings_dir = eval_path.join("recordings");
    let gt_path = eval_path.join("ground_truth.csv");

    if !gt_path.exists() {
        eprintln!(
            "Ground truth file not found: {}\nRun from the repo root or pass the eval directory as an argument.",
            gt_path.display()
        );
        std::process::exit(1);
    }

    let annotations = read_ground_truth(&gt_path);
    let params = AnalysisParams::default();

    // Group annotations by filename, preserving insertion order.
    let mut ordered_files: Vec<String> = Vec::new();
    let mut by_file: HashMap<String, Vec<&Annotation>> = HashMap::new();
    for ann in &annotations {
        let key = ann.filename.clone();
        if !by_file.contains_key(&key) {
            ordered_files.push(key.clone());
        }
        by_file.entry(key).or_default().push(ann);
    }

    // ── Per-file evaluation ───────────────────────────────────────────────────

    let mut total_tp = 0usize;
    let mut total_fn = 0usize;
    let mut total_fp = 0usize;
    let mut total_skip = 0usize;

    for filename in &ordered_files {
        let wav_path = recordings_dir.join(filename);
        let anns = &by_file[filename];

        println!("\n{}", filename);
        println!("{}", "─".repeat(filename.len()));

        if !wav_path.exists() {
            println!("  [SKIP] file not found in eval/recordings/");
            total_skip += 1;
            continue;
        }

        let result = match analyze_wav(wav_path.to_str().unwrap(), &params) {
            Ok(r) => r,
            Err(e) => {
                println!("  [ERROR] {}", e);
                total_skip += 1;
                continue;
            }
        };

        // Keep only non-dubious passes for evaluation.
        let passes: Vec<_> = result.passes.iter().filter(|p| !p.dubious).collect();

        // ── NONE annotation: expect no bats ──────────────────────────────────
        let is_no_bats = anns.len() == 1 && anns[0].expected_code == "NONE";
        if is_no_bats {
            if passes.is_empty() {
                println!("  ✓  No bats detected (correct)");
                total_tp += 1;
            } else {
                println!("  ✗  Expected no bats, but detector found:");
                for p in &passes {
                    println!(
                        "       FP  {} {:>5.1}–{:.1}s  ({} pulses)  {}",
                        p.code, p.start_sec, p.end_sec, p.n_pulses, p.species
                    );
                    total_fp += 1;
                }
                total_fn += 1;
            }
            continue;
        }

        // ── Species present: check each annotation ───────────────────────────
        let expected_codes: HashSet<&str> =
            anns.iter().map(|a| a.expected_code.as_str()).collect();

        let mut file_tp = 0usize;
        let mut file_fn = 0usize;

        for ann in anns.iter() {
            let time_str = match (ann.start_s, ann.end_s) {
                (Some(s), Some(e)) => format!(" [{:.1}–{:.1}s]", s, e),
                (Some(s), None)    => format!(" [≥{:.1}s]", s),
                (None,    Some(e)) => format!(" [≤{:.1}s]", e),
                (None,    None)    => String::new(),
            };
            let note_str = if ann.notes.is_empty() {
                String::new()
            } else {
                format!("  # {}", ann.notes)
            };

            let found = passes.iter().any(|p| {
                p.code == ann.expected_code.as_str()
                    && overlaps(p.start_sec, p.end_sec, ann.start_s, ann.end_s)
            });

            if found {
                println!("  ✓  {}{}{}", ann.expected_code, time_str, note_str);
                file_tp += 1;
            } else {
                println!("  ✗  {} expected{}{} — not found", ann.expected_code, time_str, note_str);
                file_fn += 1;
            }
        }

        // ── False positives: passes with codes not in any annotation ─────────
        let mut file_fp = 0usize;
        for p in &passes {
            if !expected_codes.contains(p.code.as_str()) {
                println!(
                    "  FP {} {:>5.1}–{:.1}s  ({} pulses)  {}",
                    p.code, p.start_sec, p.end_sec, p.n_pulses, p.species
                );
                file_fp += 1;
            }
        }

        // ── File summary ─────────────────────────────────────────────────────
        if file_fn > 0 || file_fp > 0 {
            println!(
                "     hits {}/{}  FP {}",
                file_tp,
                file_tp + file_fn,
                file_fp,
            );
        }

        total_tp += file_tp;
        total_fn += file_fn;
        total_fp += file_fp;
    }

    // ── Overall summary ───────────────────────────────────────────────────────

    let total_annotated = total_tp + total_fn;
    let sensitivity = if total_annotated > 0 {
        100.0 * total_tp as f32 / total_annotated as f32
    } else {
        0.0
    };

    println!();
    println!("══════════════════════════════════════");
    println!(
        "Sensitivity   {}/{} ({:.0}%)",
        total_tp, total_annotated, sensitivity
    );
    println!("False positives  {}", total_fp);
    if total_skip > 0 {
        println!("Skipped          {} (file not found or error)", total_skip);
    }
    println!("══════════════════════════════════════");
}
