mod classify;
mod detection;
mod features;
mod output;

use hound::WavReader;

use output::{CallGroupInfo, PeakInfo, PassInfo};

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

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let force_output = args.iter().any(|a| a == "--output");
    let path = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("Usage: bat_detector [--output] <file.wav | directory>")?;

    let meta = std::fs::metadata(path)
        .map_err(|e| format!("Cannot access '{}': {}", path, e))?;

    if meta.is_dir() {
        run_batch(path, force_output)
    } else {
        process_file(path, force_output).map(|_| ())
    }
}

// ── Batch directory mode ───────────────────────────────────────────────────────

fn run_batch(dir: &str, force_output: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Collect all WAV files directly inside `dir` (non-recursive), sorted by name.
    let mut wav_files: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read directory '{}': {}", dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".wav"))
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    wav_files.sort();

    if wav_files.is_empty() {
        println!("{}: no WAV files found", dir);
        return Ok(());
    }

    eprintln!("Batch: {} WAV files in '{}'", wav_files.len(), dir);

    let mut all_passes: Vec<(String, Vec<PassInfo>)> = Vec::new();
    let mut n_with_bats = 0usize;

    for path in &wav_files {
        match process_file(path, force_output) {
            Ok(passes) => {
                if !passes.is_empty() {
                    n_with_bats += 1;
                }
                all_passes.push((path.clone(), passes));
            }
            Err(e) => eprintln!("  skipping '{}': {}", path, e),
        }
    }

    output::write_survey_csv(dir, &all_passes)
        .map_err(|e| format!("Failed to write survey CSV: {}", e))?;

    print_batch_summary(&all_passes, wav_files.len(), n_with_bats);

    Ok(())
}

fn print_batch_summary(
    all_passes: &[(String, Vec<PassInfo>)],
    n_files: usize,
    n_with_bats: usize,
) {
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

    println!();
    println!("  {:<8}  {:<38}  {:>6}  {:>7}", "Code", "Species", "Passes", "Pulses");
    println!("  {}", "─".repeat(64));
    for ((code, species), (passes, pulses)) in &rows {
        println!("  {:<8}  {:<38}  {:>6}  {:>7}", code, species, passes, pulses);
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
) -> Result<Vec<PassInfo>, Box<dyn std::error::Error>> {
    let stem = path.trim_end_matches(".wav");

    // ── Load WAV ──────────────────────────────────────────────────────────────
    let mut reader =
        WavReader::open(path).map_err(|e| format!("Could not open '{}': {}", path, e))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Error reading samples from '{}': {}", path, e))?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Error reading samples from '{}': {}", path, e))?
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
    let noise_half_window =
        (NOISE_WINDOW_SECS * sample_rate / WINDOW_SIZE as f32).round() as usize;
    let detected = detection::detect_bat_windows(
        &spectrogram,
        bin_low,
        bin_high,
        DETECTION_THRESHOLD,
        noise_half_window,
    );

    // ── Call grouping ─────────────────────────────────────────────────────────
    let groups = detection::group_calls(&detected, GAP_FILL);

    if groups.is_empty() {
        println!("{}: NO BATS DETECTED", path);
        if !force_output {
            return Ok(vec![]);
        }
    }

    // ── Build grouped-detection mask (windows inside any call group) ──────────
    let mut grouped_detected = vec![false; n_windows];
    for &(s, e) in &groups {
        for i in s..=e {
            grouped_detected[i] = true;
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
                PeakInfo { features: f, code, species, notes }
            })
            .collect();

        let start_sec = start as f32 * WINDOW_SIZE as f32 / sample_rate;
        let end_sec = (end + 1) as f32 * WINDOW_SIZE as f32 / sample_rate;

        calls.push(CallGroupInfo {
            start_win: start,
            end_win: end,
            start_sec,
            end_sec,
            duration_ms: (end_sec - start_sec) * 1000.0,
            peaks,
        });
    }

    // ── Aggregate into species passes ─────────────────────────────────────────
    let mut passes = output::compute_passes(&calls, PASS_GAP);

    // ── Mark dubious: single-pulse passes nested inside a larger pass ─────────
    let n_passes = passes.len();
    for i in 0..n_passes {
        if passes[i].n_pulses != 1 { continue; }
        for j in 0..n_passes {
            if i == j { continue; }
            let (pi_s, pi_e) = (passes[i].start_sec, passes[i].end_sec);
            let (pj_s, pj_e, pj_n) = (passes[j].start_sec, passes[j].end_sec, passes[j].n_pulses);
            if pj_n > 1 && pi_s >= pj_s - 0.1 && pi_e <= pj_e + 0.1 {
                passes[i].dubious = true;
                break;
            }
        }
    }

    // ── Local search: sub-threshold pulses near single-pulse passes ───────────
    for i in 0..passes.len() {
        if passes[i].n_pulses != 1 || passes[i].dubious { continue; }
        let peak_hz  = passes[i].mean_peak_hz;
        let pass_spe = passes[i].species;
        let pass_s   = passes[i].start_sec;
        let pass_e   = passes[i].end_sec;

        let Some(call) = calls.iter().find(|c| {
            c.peaks.iter().any(|p| p.species == pass_spe)
                && c.start_sec <= pass_e + 0.1
                && c.end_sec   >= pass_s - 0.1
        }) else { continue };

        let band_lo = ((peak_hz - SEARCH_BAND_HZ).max(0.0) / hz_per_bin) as usize;
        let band_hi = (((peak_hz + SEARCH_BAND_HZ) / hz_per_bin).round() as usize)
            .min(freq_bins - 1);
        let n_band = (band_hi - band_lo + 1) as f32;
        let mut energy_sum = 0.0f32;
        let mut n_det = 0usize;
        for w in call.start_win..=call.end_win {
            if detected[w] {
                energy_sum += spectrogram[w][band_lo..=band_hi].iter().sum::<f32>() / n_band;
                n_det += 1;
            }
        }
        if n_det == 0 { continue; }
        let det_energy = energy_sum / n_det as f32;

        let search_wins = (SEARCH_SECS * sample_rate / WINDOW_SIZE as f32) as usize;
        let lo_win = call.start_win.saturating_sub(search_wins);
        let hi_win = (call.end_win + search_wins).min(n_windows - 1);

        passes[i].n_extra = detection::targeted_pulse_count(
            &spectrogram,
            lo_win, hi_win,
            call.start_win, call.end_win,
            peak_hz, hz_per_bin, SEARCH_BAND_HZ,
            det_energy, LOCAL_SEARCH_THRESH,
        );
    }

    // ── Per-pass energy (dB re FFT² units, comparable across files) ──────────
    for pass in &mut passes {
        let win_start = (pass.start_sec * sample_rate / WINDOW_SIZE as f32) as usize;
        let win_end   = ((pass.end_sec * sample_rate / WINDOW_SIZE as f32) as usize)
            .min(n_windows - 1);
        let mut energy_sum = 0.0f32;
        let mut peak_energy = 0.0f32;
        let mut n_det = 0usize;
        for w in win_start..=win_end {
            if detected[w] {
                let e = spectrogram[w][bin_low..=bin_high].iter().sum::<f32>()
                    / (bin_high - bin_low + 1) as f32;
                energy_sum += e;
                if e > peak_energy { peak_energy = e; }
                n_det += 1;
            }
        }
        if n_det > 0 {
            let mean_e = energy_sum / n_det as f32;
            pass.mean_energy_db = if mean_e > 0.0 { 10.0 * mean_e.log10() } else { -120.0 };
            pass.peak_energy_db = if peak_energy > 0.0 { 10.0 * peak_energy.log10() } else { -120.0 };
        }
    }

    // ── Print pass summary ────────────────────────────────────────────────────
    for (i, pass) in passes.iter().enumerate() {
        let extra = if pass.n_extra > 0 {
            format!(", +{} nearby", pass.n_extra)
        } else {
            String::new()
        };
        let flag = if pass.dubious { " [dubious: nested]" } else { "" };
        println!(
            "{}: pass {} {:.1}–{:.1}s ({} pulse{}{}) → {} - {}{}",
            path, i + 1,
            pass.start_sec, pass.end_sec,
            pass.n_pulses,
            if pass.n_pulses == 1 { "" } else { "s" },
            extra,
            pass.code,
            pass.species,
            flag,
        );
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

    // ── Outputs ───────────────────────────────────────────────────────────────
    output::write_csv(stem, path, &passes)
        .map_err(|e| format!("Failed to write CSV for '{}': {}", stem, e))?;

    output::write_png(stem, &spec_bytes, &grouped_detected, n_windows, freq_bins, bin_low, bin_high)
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
