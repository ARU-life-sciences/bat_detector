mod classify;
mod detection;
mod features;
mod output;

use hound::WavReader;

use output::{CallGroupInfo, PeakInfo};

const BAT_FREQ_LOW_HZ: f32 = 20_000.0;
const BAT_FREQ_HIGH_HZ: f32 = 120_000.0;
const ID_FREQ_LOW_HZ: f32 = 18_000.0;
const ENERGY_THRESHOLD: f32 = 0.01;
const WINDOW_SIZE: usize = 1024;
const GAP_FILL: usize = 10;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: bat_detector <file.wav>");
    let stem = path.trim_end_matches(".wav");

    // ── Load WAV ──────────────────────────────────────────────────────────────
    let mut reader = WavReader::open(&path).expect("Failed to open WAV file");
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>().map(|s| s.unwrap()).collect()
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / max).collect()
        }
    };

    println!(
        "Loaded: {} samples at {} Hz ({:.2} s)",
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

    // ── Detection pass ────────────────────────────────────────────────────────
    let windows = detection::process(
        &samples,
        sample_rate,
        WINDOW_SIZE,
        bin_low,
        bin_high,
        ENERGY_THRESHOLD,
    );
    let n_windows = windows.len();
    println!("Windows: {}", n_windows);

    // ── Split into spectrogram + detection vectors ────────────────────────────
    let detected: Vec<bool> = windows.iter().map(|w| w.is_bat).collect();
    let spectrogram: Vec<Vec<f32>> = windows.into_iter().map(|w| w.power).collect();

    // ── Call grouping ─────────────────────────────────────────────────────────
    let groups = detection::group_calls(&detected, GAP_FILL);

    // ── Per-group feature extraction + classification ─────────────────────────
    let mut calls: Vec<CallGroupInfo> = Vec::new();
    for (start, end) in &groups {
        let start = *start;
        let end = *end;

        let all_features = features::extract_call_features(
            &spectrogram,
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
                let (species, notes) = classify::classify_british(&f);
                PeakInfo { features: f, species, notes }
            })
            .collect();

        let start_sec = start as f32 * WINDOW_SIZE as f32 / sample_rate;
        let end_sec = (end + 1) as f32 * WINDOW_SIZE as f32 / sample_rate;

        println!(
            "Call group {}: {:.3}–{:.3} s ({} peak(s))",
            calls.len() + 1,
            start_sec,
            end_sec,
            peaks.len()
        );
        for p in &peaks {
            println!("  → {} | {}", p.species, p.notes);
        }

        calls.push(CallGroupInfo {
            start_win: start,
            end_win: end,
            start_sec,
            end_sec,
            duration_ms: (end_sec - start_sec) * 1000.0,
            peaks,
        });
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
                ((db - noise_floor_db) / (-noise_floor_db) * 255.0)
                    .clamp(0.0, 255.0) as u8
            })
        })
        .collect();

    // ── Outputs ───────────────────────────────────────────────────────────────
    output::write_png(stem, &spec_bytes, &detected, n_windows, freq_bins, bin_low, bin_high)
        .expect("Failed to write PNG");

    output::write_html(
        stem,
        sample_rate,
        WINDOW_SIZE,
        n_windows,
        freq_bins,
        hz_per_bin,
        &spec_bytes,
        &detected,
        &calls,
    )
    .expect("Failed to write HTML");
}
