mod classify;
mod detection;
mod features;
mod output;

use hound::WavReader;
use rayon::prelude::*;

use output::{CallGroupInfo, PassInfo, PeakInfo};

const BAT_FREQ_LOW_HZ: f32 = 20_000.0;
const BAT_FREQ_HIGH_HZ: f32 = 120_000.0;
const ID_FREQ_LOW_HZ: f32 = 18_000.0;
/// A window is flagged as bat activity when its mean bat-band energy exceeds
/// the local 10th-percentile noise floor by this factor.  Raise to reduce
/// false positives in noisy recordings; lower to catch distant/faint calls.
const DETECTION_THRESHOLD: f32 = 3.0;
/// Half-width (seconds) of the rolling window used to estimate the local noise
/// floor.  Wider → more stable estimate; narrower → faster adaptation to
/// sudden changes in background noise (e.g. intermittent insect bursts).
const NOISE_WINDOW_SECS: f32 = 3.0;
const WINDOW_SIZE: usize = 1024;
const GAP_FILL: usize = 25;
/// Maximum gap (seconds) between consecutive same-species groups to merge into one pass.
const PASS_GAP: f32 = 2.0;
/// Half-width of the search window (seconds) around a single-pulse detection.
const SEARCH_SECS: f32 = 1.0;
/// Half-width of the frequency band (Hz) used in the local pulse search.
const SEARCH_BAND_HZ: f32 = 5_000.0;
/// Secondary detection threshold as a fraction of the detected pulse's band energy.
const LOCAL_SEARCH_THRESH: f32 = 0.3;

/// Minimum −10 dB bandwidth for a non-CF call group to be kept (Hz).
///
/// Genuine FM bat calls sweep across at least 5–30 kHz so their mean spectrum
/// is broad.  Narrowband interference (electronic tones, machinery harmonics)
/// concentrates energy in a very thin slice and is rejected here.  Horseshoe-bat
/// CF calls (is_cf = true) are exempt — they are intentionally narrowband.
const MIN_FM_BANDWIDTH_HZ: f32 = 3_000.0;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let force_output = args.iter().any(|a| a == "--output");
    let diagnose = args.iter().any(|a| a == "--diagnose");

    // --threshold <value>  overrides DETECTION_THRESHOLD at runtime.
    let threshold = if let Some(pos) = args.iter().position(|a| a == "--threshold") {
        args.get(pos + 1)
            .ok_or("--threshold requires a value")?
            .parse::<f32>()
            .map_err(|_| "--threshold value must be a number (e.g. --threshold 5.0)")?
    } else {
        DETECTION_THRESHOLD
    };

    // --ratio <value>  overrides SPECTRAL_RATIO_MIN at runtime.
    // Lower values (e.g. 1.0) pass calls in recordings with broadband noise.
    let ratio = if let Some(pos) = args.iter().position(|a| a == "--ratio") {
        args.get(pos + 1)
            .ok_or("--ratio requires a value")?
            .parse::<f32>()
            .map_err(|_| "--ratio value must be a number (e.g. --ratio 1.0)")?
    } else {
        detection::SPECTRAL_RATIO_MIN_DEFAULT
    };

    let threshold_val_pos = args.iter().position(|a| a == "--threshold").map(|p| p + 1);
    let ratio_val_pos = args.iter().position(|a| a == "--ratio").map(|p| p + 1);
    let path = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with('-')
                && Some(*i) != threshold_val_pos
                && Some(*i) != ratio_val_pos
        })
        .map(|(_, a)| a)
        .ok_or("Usage: bat_detector [--output] [--threshold <n>] [--ratio <n>] [--diagnose] <file.wav | directory>")?;

    let meta = std::fs::metadata(path).map_err(|e| format!("Cannot access '{}': {}", path, e))?;

    if meta.is_dir() {
        run_batch(path, force_output, threshold, ratio, diagnose)
    } else {
        process_file(path, force_output, threshold, ratio, false, diagnose).map(|_| ())
    }
}

// ── Batch directory mode ───────────────────────────────────────────────────────

fn run_batch(
    dir: &str,
    force_output: bool,
    threshold: f32,
    ratio: f32,
    diagnose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect all WAV files directly inside `dir` (non-recursive), sorted by name.
    let mut wav_files: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read directory '{}': {}", dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".wav")
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    wav_files.sort();

    if wav_files.is_empty() {
        println!("{}: no WAV files found", dir);
        return Ok(());
    }

    let n_threads = rayon::current_num_threads();
    eprintln!(
        "Batch: {} WAV files in '{}' ({} threads)",
        wav_files.len(),
        dir,
        n_threads
    );

    // Process files in parallel; results are collected in original filename order.
    // Per-pass detail lines are suppressed (quiet=true) — interleaved output from
    // multiple threads would be unreadable; use the per-file HTML/CSV for details.
    let results: Vec<(String, Result<Vec<PassInfo>, String>)> = wav_files
        .par_iter()
        .map(|path| {
            let r = process_file(path, force_output, threshold, ratio, true, diagnose)
                .map_err(|e| e.to_string());
            (path.clone(), r)
        })
        .collect();

    let mut all_passes: Vec<(String, Vec<PassInfo>)> = Vec::new();
    let mut n_with_bats = 0usize;

    for (path, result) in results {
        match result {
            Ok(passes) => {
                if !passes.is_empty() {
                    n_with_bats += 1;
                }
                all_passes.push((path, passes));
            }
            Err(e) => eprintln!("  skipping '{}': {}", path, e),
        }
    }

    output::write_survey_csv(dir, &all_passes)
        .map_err(|e| format!("Failed to write survey CSV: {}", e))?;

    print_batch_summary(&all_passes, wav_files.len(), n_with_bats);

    Ok(())
}

fn print_batch_summary(all_passes: &[(String, Vec<PassInfo>)], n_files: usize, n_with_bats: usize) {
    use std::collections::HashMap;

    // Accumulate per-species pass and pulse counts (dubious passes excluded).
    let mut by_species: HashMap<(&str, &str), (usize, usize)> = HashMap::new();
    for (_, passes) in all_passes {
        for pass in passes {
            if !pass.dubious {
                let e = by_species.entry((pass.code, pass.species)).or_default();
                e.0 += 1;
                e.1 += pass.n_pulses;
            }
        }
    }

    println!("\n── Batch summary ──────────────────────────────────────────────");
    println!(
        "  Files processed : {}  ({} with bat activity)",
        n_files, n_with_bats
    );

    if by_species.is_empty() {
        println!("  No bat detections.");
        println!("───────────────────────────────────────────────────────────────");
        return;
    }

    let mut rows: Vec<_> = by_species.iter().collect();
    // Sort descending by pass count, then alphabetically by species name.
    rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.1.cmp(b.0.1)));

    const SP_W: usize = 38;
    let truncate = |s: &str| -> String {
        if s.chars().count() <= SP_W {
            s.to_string()
        } else {
            let t: String = s.chars().take(SP_W - 1).collect();
            format!("{}…", t)
        }
    };

    println!();
    println!(
        "  {:<8}  {:<38}  {:>6}  {:>7}",
        "Code", "Species", "Passes", "Pulses"
    );
    println!("  {}", "─".repeat(64));
    for ((code, species), (passes, pulses)) in &rows {
        println!(
            "  {:<8}  {:<38}  {:>6}  {:>7}",
            code,
            truncate(species),
            passes,
            pulses
        );
    }
    println!("───────────────────────────────────────────────────────────────");
}

// ── Single-file processing ─────────────────────────────────────────────────────

/// Process one WAV file: detect, classify, write per-file outputs (CSV, PNG, HTML).
/// Returns the list of species passes found (empty when no bats detected and
/// `force_output` is false).
fn process_file(
    path: &str,
    force_output: bool,
    threshold: f32,
    ratio: f32,
    quiet: bool,
    diagnose: bool,
) -> Result<Vec<PassInfo>, Box<dyn std::error::Error>> {
    let stem = path.trim_end_matches(".wav");

    // ── Load WAV ──────────────────────────────────────────────────────────────
    let mut reader =
        WavReader::open(path).map_err(|e| format!("Could not open '{}': {}", path, e))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;

    // Collect samples, stopping at the first read error rather than failing.
    // This makes the reader tolerant of truncated WAV files where the data
    // chunk header claims more bytes than are actually present on disk.
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            let mut v = Vec::new();
            for s in reader.samples::<f32>() {
                match s { Ok(x) => v.push(x), Err(_) => break }
            }
            v
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            let mut v = Vec::new();
            for s in reader.samples::<i32>() {
                match s { Ok(x) => v.push(x as f32 / max), Err(_) => break }
            }
            v
        }
    };

    eprintln!(
        "{}: loaded {} samples at {} Hz ({:.2} s)",
        path,
        samples.len(),
        sample_rate as u32,
        samples.len() as f32 / sample_rate
    );

    // ── Frequency bin layout ──────────────────────────────────────────────────
    let freq_bins = WINDOW_SIZE / 2;
    let hz_per_bin = sample_rate / WINDOW_SIZE as f32;
    let bin_low = (BAT_FREQ_LOW_HZ / hz_per_bin).round() as usize;
    let bin_high = ((BAT_FREQ_HIGH_HZ / hz_per_bin).round() as usize).min(freq_bins - 1);
    let bin_id_low = (ID_FREQ_LOW_HZ / hz_per_bin).round() as usize;

    // ── FFT spectrogram ───────────────────────────────────────────────────────
    let spectrogram = detection::compute_spectrogram(&samples, WINDOW_SIZE);
    let n_windows = spectrogram.len();

    // ── Adaptive bat-window detection ─────────────────────────────────────────
    let noise_half_window = (NOISE_WINDOW_SECS * sample_rate / WINDOW_SIZE as f32).round() as usize;
    let detected = detection::detect_bat_windows(
        &spectrogram,
        bin_low,
        bin_high,
        threshold,
        noise_half_window,
        ratio,
    );

    // ── Call grouping ─────────────────────────────────────────────────────────
    let groups = detection::group_calls(&detected, GAP_FILL);

    if groups.is_empty() {
        println!("{}: NO BATS DETECTED", path);
        if !force_output {
            return Ok(vec![]);
        }
    }

    // ── Per-group feature extraction + classification ─────────────────────────
    let mut calls: Vec<CallGroupInfo> = Vec::new();
    for (start, end) in &groups {
        let start = *start;
        let end = *end;

        let all_features = features::extract_call_features(
            &spectrogram,
            &detected,
            start,
            end,
            bin_id_low,
            bin_low,
            bin_high,
            freq_bins,
            hz_per_bin,
            sample_rate,
            WINDOW_SIZE,
        );

        let peaks: Vec<PeakInfo> = all_features
            .into_iter()
            .map(|f| {
                let (code, species, notes) = classify::classify_british(&f);
                PeakInfo {
                    features: f,
                    code,
                    species,
                    notes,
                }
            })
            .collect();

        let start_sec = start as f32 * WINDOW_SIZE as f32 / sample_rate;
        let end_sec = (end + 1) as f32 * WINDOW_SIZE as f32 / sample_rate;

        calls.push(CallGroupInfo {
            start_win: start,
            end_win: end,
            start_sec,
            end_sec,
            peaks,
        });
    }

    // ── Bandwidth gate: reject narrowband interference ────────────────────────
    // Genuine FM bat calls sweep across several kHz.  The −20 dB frequency range
    // (freq_high − freq_low) captures the full sweep extent even for short call
    // groups where the mean spectrum is dominated by the CF tail.  The −10 dB
    // bandwidth is too conservative for those cases.  CF calls (horseshoe bats,
    // is_cf = true) are exempt — they are intentionally narrowband.
    calls.retain(|c| {
        c.peaks.iter().any(|p| {
            p.features.is_cf
                || (p.features.freq_high_hz - p.features.freq_low_hz) >= MIN_FM_BANDWIDTH_HZ
        })
    });

    // ── Build grouped-detection mask from all raw groups ─────────────────────
    // Built from `groups` (pre-bandwidth-gate) so every detected window appears
    // highlighted in the spectrogram.  The bandwidth gate only controls which
    // groups receive a table row; it does not suppress the visual highlight.
    let mut grouped_detected = vec![false; n_windows];
    for &(s, e) in &groups {
        grouped_detected[s..=e].fill(true);
    }

    // ── Aggregate into species passes ─────────────────────────────────────────
    let mut passes = output::compute_passes(&calls, PASS_GAP);

    // ── Mark dubious: single-pulse passes nested inside a larger pass ─────────
    let n_passes = passes.len();
    for i in 0..n_passes {
        if passes[i].n_pulses != 1 {
            continue;
        }
        for j in 0..n_passes {
            if i == j {
                continue;
            }
            // only suppress passes when they are the same species
            if passes[i].species != passes[j].species {
                continue;
            }
            let (pi_s, pi_e) = (passes[i].start_sec, passes[i].end_sec);
            let (pj_s, pj_e, pj_n) = (passes[j].start_sec, passes[j].end_sec, passes[j].n_pulses);
            if pj_n > 1 && pi_s >= pj_s - 0.1 && pi_e <= pj_e + 0.1 {
                passes[i].dubious = true;
                break;
            }
        }
    }

    // ── Mark dubious + absorb: overlapping multi-pulse passes ─────────────────
    // When ≥50% of a pass's duration is covered by another pass with ≥2× as many
    // pulses, the weaker pass is classification noise from the same bat sequence.
    // We flag it dubious AND absorb it into the dominant pass: the dominant pass's
    // time range is extended to cover the absorbed pass and its pulse count grows,
    // giving a more accurate time span and a higher confidence score.
    //
    // Collect (dominated, dominant) index pairs first so we can batch-apply the
    // extensions without the in-progress modifications affecting the comparisons.
    let mut absorb: Vec<(usize, usize)> = Vec::new();
    for i in 0..n_passes {
        if passes[i].dubious {
            continue;
        }
        let (pi_s, pi_e, pi_n) = (passes[i].start_sec, passes[i].end_sec, passes[i].n_pulses);
        let pi_dur = (pi_e - pi_s).max(1e-6);
        for j in 0..n_passes {
            // only suppress passes when they are the same species
            if i == j {
                continue;
            }
            if passes[i].species != passes[j].species {
                continue;
            }
            if passes[j].n_pulses < 2 * pi_n {
                continue;
            }
            let (pj_s, pj_e) = (passes[j].start_sec, passes[j].end_sec);
            let overlap = (pi_e.min(pj_e) - pi_s.max(pj_s)).max(0.0);
            if overlap / pi_dur >= 0.5 {
                passes[i].dubious = true;
                absorb.push((i, j));
                break;
            }
        }
    }
    for (i, j) in absorb {
        let (pi_s, pi_e, pi_n) = (passes[i].start_sec, passes[i].end_sec, passes[i].n_pulses);
        if pi_s < passes[j].start_sec {
            passes[j].start_sec = pi_s;
        }
        if pi_e > passes[j].end_sec {
            passes[j].end_sec = pi_e;
        }
        passes[j].n_pulses += pi_n;
    }

    // ── Feeding-buzz labelling ────────────────────────────────────────────────
    output::flag_feeding_buzzes(&mut passes);

    // ── Local search: sub-threshold pulses near single-pulse passes ───────────
    for i in 0..passes.len() {
        if passes[i].n_pulses != 1 || passes[i].dubious {
            continue;
        }
        let peak_hz = passes[i].mean_peak_hz;
        let pass_spe = passes[i].species;
        let pass_s = passes[i].start_sec;
        let pass_e = passes[i].end_sec;

        let Some(call) = calls.iter().find(|c| {
            c.peaks.iter().any(|p| p.species == pass_spe)
                && c.start_sec <= pass_e + 0.1
                && c.end_sec >= pass_s - 0.1
        }) else {
            continue;
        };

        let band_lo = ((peak_hz - SEARCH_BAND_HZ).max(0.0) / hz_per_bin) as usize;
        let band_hi =
            (((peak_hz + SEARCH_BAND_HZ) / hz_per_bin).round() as usize).min(freq_bins - 1);
        let n_band = (band_hi - band_lo + 1) as f32;
        let mut energy_sum = 0.0f32;
        let mut n_det = 0usize;
        for w in call.start_win..=call.end_win {
            if detected[w] {
                energy_sum += spectrogram[w][band_lo..=band_hi].iter().sum::<f32>() / n_band;
                n_det += 1;
            }
        }
        if n_det == 0 {
            continue;
        }
        let det_energy = energy_sum / n_det as f32;

        let search_wins = (SEARCH_SECS * sample_rate / WINDOW_SIZE as f32) as usize;
        let lo_win = call.start_win.saturating_sub(search_wins);
        let hi_win = (call.end_win + search_wins).min(n_windows - 1);

        let n_extra = detection::targeted_pulse_count(
            &spectrogram,
            lo_win,
            hi_win,
            call.start_win,
            call.end_win,
            peak_hz,
            hz_per_bin,
            SEARCH_BAND_HZ,
            det_energy,
            LOCAL_SEARCH_THRESH,
        );

        // Only credit the nearby pulses if no other species' pass overlaps the
        // ±SEARCH_SECS window.  Even a single-pulse pass of a different species
        // is enough to disqualify: the frequency-band search may be picking up
        // that species' pulses (which can be close in frequency), so we must not
        // credit them as sub-threshold evidence for this bat.
        let search_t0 = pass_s - SEARCH_SECS;
        let search_t1 = pass_e + SEARCH_SECS;
        let other_bat_nearby = passes.iter().enumerate().any(|(j, p)| {
            j != i
                && !p.dubious
                && p.species != pass_spe
                && p.start_sec < search_t1
                && p.end_sec > search_t0
        });
        passes[i].n_extra = if other_bat_nearby { 0 } else { n_extra };
    }

    // ── Per-pass energy (dB re FFT² units, comparable across files) ──────────
    for pass in &mut passes {
        let win_start = (pass.start_sec * sample_rate / WINDOW_SIZE as f32) as usize;
        let win_end =
            ((pass.end_sec * sample_rate / WINDOW_SIZE as f32) as usize).min(n_windows - 1);
        let mut energy_sum = 0.0f32;
        let mut peak_energy = 0.0f32;
        let mut n_det = 0usize;
        for w in win_start..=win_end {
            if detected[w] {
                let e = spectrogram[w][bin_low..=bin_high].iter().sum::<f32>()
                    / (bin_high - bin_low + 1) as f32;
                energy_sum += e;
                if e > peak_energy {
                    peak_energy = e;
                }
                n_det += 1;
            }
        }
        if n_det > 0 {
            let mean_e = energy_sum / n_det as f32;
            pass.mean_energy_db = if mean_e > 0.0 {
                10.0 * mean_e.log10()
            } else {
                -120.0
            };
            pass.peak_energy_db = if peak_energy > 0.0 {
                10.0 * peak_energy.log10()
            } else {
                -120.0
            };
        }
    }

    // ── Print pass summary (suppressed in batch/quiet mode) ──────────────────
    if !quiet {
        for (i, pass) in passes.iter().enumerate() {
            let extra = if pass.n_extra > 0 {
                format!(", +{} nearby", pass.n_extra)
            } else {
                String::new()
            };
            let flag = if pass.dubious {
                if pass.n_pulses == 1 {
                    " [dubious: nested]"
                } else {
                    " [dubious: overlapping]"
                }
            } else {
                ""
            };
            println!(
                "{}: pass {} {:.1}–{:.1}s ({} pulse{}{}) → {} - {}{}",
                path,
                i + 1,
                pass.start_sec,
                pass.end_sec,
                pass.n_pulses,
                if pass.n_pulses == 1 { "" } else { "s" },
                extra,
                pass.code,
                pass.species,
                flag,
            );
        }
    }

    // ── dB normalisation → spec_bytes ─────────────────────────────────────────
    let noise_floor_db: f32 = -80.0;
    let max_power = spectrogram
        .iter()
        .flat_map(|w| w.iter())
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);

    let spec_bytes: Vec<u8> = spectrogram
        .iter()
        .flat_map(|w| {
            w.iter().map(|&p| {
                if p <= 0.0 || max_power <= 0.0 {
                    return 0u8;
                }
                let db = 10.0 * (p / max_power).log10();
                ((db - noise_floor_db) / (-noise_floor_db) * 255.0).clamp(0.0, 255.0) as u8
            })
        })
        .collect();

    // ── Diagnostic TSV (--diagnose) ───────────────────────────────────────────
    if diagnose {
        let diags = detection::detect_bat_windows_diag(
            &spectrogram,
            bin_low,
            bin_high,
            threshold,
            noise_half_window,
            ratio,
            sample_rate,
            WINDOW_SIZE,
        );
        let tsv_path = format!("{}_detection_diag.tsv", stem);
        let mut f = std::fs::File::create(&tsv_path)
            .map_err(|e| format!("Cannot create '{}': {}", tsv_path, e))?;
        use std::io::Write as _;
        writeln!(
            f,
            "time_s\tbat_max\tnoise_floor\tcond1_ratio\tbat_mean\tnonbat_mean\tcond2_ratio\tcond1_pass\tcond2_pass\tdetected"
        )?;
        for d in &diags {
            writeln!(
                f,
                "{:.4}\t{:.6e}\t{:.6e}\t{:.4}\t{:.6e}\t{:.6e}\t{:.4}\t{}\t{}\t{}",
                d.time_s,
                d.bat_max,
                d.noise_floor,
                d.cond1_ratio,
                d.bat_mean,
                d.nonbat_mean,
                d.cond2_ratio,
                d.cond1_pass as u8,
                d.cond2_pass as u8,
                d.detected as u8,
            )?;
        }
        eprintln!("Wrote diagnostic TSV: {}", tsv_path);
    }

    // ── Outputs ───────────────────────────────────────────────────────────────
    output::write_csv(stem, path, &passes)
        .map_err(|e| format!("Failed to write CSV for '{}': {}", stem, e))?;

    output::write_png(
        stem,
        &spec_bytes,
        &grouped_detected,
        n_windows,
        freq_bins,
        bin_low,
        bin_high,
    )
    .map_err(|e| format!("Failed to write PNG for '{}': {}", stem, e))?;

    output::write_html(
        stem,
        sample_rate,
        WINDOW_SIZE,
        n_windows,
        freq_bins,
        hz_per_bin,
        &spec_bytes,
        &grouped_detected,
        &calls,
        &passes,
    )
    .map_err(|e| format!("Failed to write HTML for '{}': {}", stem, e))?;

    Ok(passes)
}
