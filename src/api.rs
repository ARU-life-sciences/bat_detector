//! Public library API for GUI consumers (e.g. the Tauri bat-review app).
//!
//! [`analyze_wav`] runs the full detection/classification pipeline on a WAV
//! file and returns a [`RecordingResult`] that contains:
//!
//! * [`PassRecord`] — serialisable pass data for display in an editable table.
//! * A pre-built HTML spectrogram viewer (via [`RecordingResult::to_html`]).

use serde::Serialize;
use std::path::Path;

use hound::WavReader;

use crate::output::{CallGroupInfo, PassInfo, PeakInfo};
use crate::{classify, detection, features, output};

// ── Analysis parameters ───────────────────────────────────────────────────────

const BAT_FREQ_LOW_HZ: f32  = 20_000.0;
const BAT_FREQ_HIGH_HZ: f32 = 120_000.0;
const ID_FREQ_LOW_HZ: f32   = 18_000.0;
const DETECTION_THRESHOLD: f32 = 3.0;
const NOISE_WINDOW_SECS: f32   = 3.0;
const WINDOW_SIZE: usize       = 1024;
const GAP_FILL: usize          = 25;
const PASS_GAP: f32            = 2.0;
const SEARCH_SECS: f32         = 1.0;
const SEARCH_BAND_HZ: f32      = 5_000.0;
const LOCAL_SEARCH_THRESH: f32 = 0.3;
const MIN_FM_BANDWIDTH_HZ: f32 = 3_000.0;

/// Tunable detection parameters (all have sensible defaults via [`Default`]).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct AnalysisParams {
    /// Adaptive noise-floor multiplier (default 3.0).
    pub threshold: f32,
    /// Minimum bat-band / non-bat-band spectral ratio (default 1.05).
    pub ratio: f32,
}

impl Default for AnalysisParams {
    fn default() -> Self {
        Self {
            threshold: DETECTION_THRESHOLD,
            ratio: detection::SPECTRAL_RATIO_MIN_DEFAULT,
        }
    }
}

// ── Serialisable output types ─────────────────────────────────────────────────

/// One species pass — serialisable subset of [`PassInfo`] for the review table.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PassRecord {
    pub idx: usize,
    pub code: String,
    pub species: String,
    pub notes: String,
    pub start_sec: f32,
    pub end_sec: f32,
    pub n_pulses: usize,
    pub n_extra: usize,
    pub mean_peak_khz: f32,
    pub peak_hz_std_khz: f32,
    pub freq_low_khz: f32,
    pub freq_high_khz: f32,
    pub bandwidth_khz: f32,
    pub cf_tail_ratio: f32,
    pub rep_rate: f32,
    pub is_cf: bool,
    pub call_dur_ms: f32,
    pub mean_energy_db: f32,
    pub peak_energy_db: f32,
    pub dubious: bool,
    pub confidence: f32,
}

impl PassRecord {
    fn from_pass(idx: usize, p: &PassInfo) -> Self {
        Self {
            idx,
            code:            p.code.to_string(),
            species:         p.species.to_string(),
            notes:           p.notes.to_string(),
            start_sec:       p.start_sec,
            end_sec:         p.end_sec,
            n_pulses:        p.n_pulses,
            n_extra:         p.n_extra,
            mean_peak_khz:   p.mean_peak_hz   / 1000.0,
            peak_hz_std_khz: p.peak_hz_std    / 1000.0,
            freq_low_khz:    p.mean_freq_low_hz  / 1000.0,
            freq_high_khz:   p.mean_freq_high_hz / 1000.0,
            bandwidth_khz:   p.mean_bandwidth_hz / 1000.0,
            cf_tail_ratio:   p.mean_cf_tail_ratio,
            rep_rate:        p.mean_rep_rate,
            is_cf:           p.is_cf,
            call_dur_ms:     p.mean_call_duration_ms,
            mean_energy_db:  p.mean_energy_db,
            peak_energy_db:  p.peak_energy_db,
            dubious:         p.dubious,
            confidence:      p.confidence(),
        }
    }
}

// ── Recording result ──────────────────────────────────────────────────────────

/// Full analysis result for one WAV file.
pub struct RecordingResult {
    /// Filename (no directory component).
    pub file_name: String,
    pub sample_rate: u32,
    pub duration_sec: f32,
    /// Serialisable pass records for the review grid.
    pub passes: Vec<PassRecord>,

    // Internal fields used by to_html() — not serialised directly.
    stem:        String,
    n_windows:   usize,
    freq_bins:   usize,
    hz_per_bin:  f32,
    window_size: usize,
    spec_bytes:  Vec<u8>,
    detected:    Vec<bool>,
    calls:       Vec<CallGroupInfo>,
    raw_passes:  Vec<PassInfo>,
}

impl RecordingResult {
    /// Build the self-contained spectrogram HTML string for embedding in an iframe.
    pub fn to_html(&self) -> String {
        let mut buf: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        output::write_html_to(
            &mut buf,
            &self.stem,
            self.sample_rate as f32,
            self.window_size,
            self.n_windows,
            self.freq_bins,
            self.hz_per_bin,
            &self.spec_bytes,
            &self.detected,
            &self.calls,
            &self.raw_passes,
        )
        .expect("in-memory write cannot fail");
        String::from_utf8(buf).expect("HTML is valid UTF-8")
    }
}

// ── Analysis pipeline ─────────────────────────────────────────────────────────

/// Run the full bat detection pipeline on `path` and return a [`RecordingResult`].
///
/// This is equivalent to the CLI's single-file mode but returns structured data
/// instead of writing CSV / PNG / HTML files.
pub fn analyze_wav(path: &str, params: &AnalysisParams) -> Result<RecordingResult, String> {
    let wav_path = Path::new(path);
    let stem = wav_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording")
        .to_string();
    let file_name = wav_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string();

    // ── Load WAV ──────────────────────────────────────────────────────────────
    let mut reader =
        WavReader::open(path).map_err(|e| format!("Could not open '{}': {}", path, e))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate as f32;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Error reading samples: {}", e))?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Error reading samples: {}", e))?
        }
    };

    // ── Frequency bin layout ──────────────────────────────────────────────────
    let freq_bins   = WINDOW_SIZE / 2;
    let hz_per_bin  = sample_rate / WINDOW_SIZE as f32;
    let bin_low     = (BAT_FREQ_LOW_HZ  / hz_per_bin).round() as usize;
    let bin_high    = ((BAT_FREQ_HIGH_HZ / hz_per_bin).round() as usize).min(freq_bins - 1);
    let bin_id_low  = (ID_FREQ_LOW_HZ   / hz_per_bin).round() as usize;

    // ── FFT spectrogram ───────────────────────────────────────────────────────
    let spectrogram = detection::compute_spectrogram(&samples, WINDOW_SIZE);
    let n_windows   = spectrogram.len();

    // ── Adaptive bat-window detection ─────────────────────────────────────────
    let noise_half_window =
        (NOISE_WINDOW_SECS * sample_rate / WINDOW_SIZE as f32).round() as usize;
    let detected_raw = detection::detect_bat_windows(
        &spectrogram,
        bin_low,
        bin_high,
        params.threshold,
        noise_half_window,
        params.ratio,
    );

    // ── Call grouping ─────────────────────────────────────────────────────────
    let groups = detection::group_calls(&detected_raw, GAP_FILL);

    // ── Per-group feature extraction + classification ─────────────────────────
    let mut calls: Vec<CallGroupInfo> = Vec::new();
    for (start, end) in &groups {
        let (start, end) = (*start, *end);
        let all_features = features::extract_call_features(
            &spectrogram, &detected_raw,
            start, end,
            bin_id_low, bin_low, bin_high, freq_bins, hz_per_bin, sample_rate, WINDOW_SIZE,
        );
        let peaks: Vec<PeakInfo> = all_features
            .into_iter()
            .map(|f| {
                let (code, species, notes) = classify::classify_british(&f);
                PeakInfo { features: f, code, species, notes }
            })
            .collect();
        let start_sec = start as f32 * WINDOW_SIZE as f32 / sample_rate;
        let end_sec   = (end + 1) as f32 * WINDOW_SIZE as f32 / sample_rate;
        calls.push(CallGroupInfo { start_win: start, end_win: end, start_sec, end_sec, peaks });
    }

    // ── Bandwidth gate ────────────────────────────────────────────────────────
    calls.retain(|c| {
        c.peaks.iter().any(|p| {
            p.features.is_cf
                || (p.features.freq_high_hz - p.features.freq_low_hz) >= MIN_FM_BANDWIDTH_HZ
        })
    });

    // ── Grouped detection mask (pre-gate, for spectrogram highlight) ──────────
    let mut grouped_detected = vec![false; n_windows];
    for &(s, e) in &groups {
        grouped_detected[s..=e].fill(true);
    }

    // ── Aggregate into species passes ─────────────────────────────────────────
    let mut passes = output::compute_passes(&calls, PASS_GAP);
    let n_passes = passes.len();

    // ── Mark dubious: nested single-pulse passes ──────────────────────────────
    for i in 0..n_passes {
        if passes[i].n_pulses != 1 { continue; }
        for j in 0..n_passes {
            if i == j || passes[i].species != passes[j].species { continue; }
            let (pi_s, pi_e) = (passes[i].start_sec, passes[i].end_sec);
            let (pj_s, pj_e, pj_n) = (passes[j].start_sec, passes[j].end_sec, passes[j].n_pulses);
            if pj_n > 1 && pi_s >= pj_s - 0.1 && pi_e <= pj_e + 0.1 {
                passes[i].dubious = true;
                break;
            }
        }
    }

    // ── Mark dubious + absorb: overlapping multi-pulse passes ─────────────────
    let mut absorb: Vec<(usize, usize)> = Vec::new();
    for i in 0..n_passes {
        if passes[i].dubious { continue; }
        let (pi_s, pi_e, pi_n) = (passes[i].start_sec, passes[i].end_sec, passes[i].n_pulses);
        let pi_dur = (pi_e - pi_s).max(1e-6);
        for j in 0..n_passes {
            if i == j || passes[i].species != passes[j].species { continue; }
            if passes[j].n_pulses < 2 * pi_n { continue; }
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
        if pi_s < passes[j].start_sec { passes[j].start_sec = pi_s; }
        if pi_e > passes[j].end_sec   { passes[j].end_sec   = pi_e; }
        passes[j].n_pulses += pi_n;
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
        }) else { continue; };

        let band_lo = ((peak_hz - SEARCH_BAND_HZ).max(0.0) / hz_per_bin) as usize;
        let band_hi = (((peak_hz + SEARCH_BAND_HZ) / hz_per_bin).round() as usize).min(freq_bins - 1);
        let n_band  = (band_hi - band_lo + 1) as f32;
        let mut energy_sum = 0.0f32;
        let mut n_det = 0usize;
        for w in call.start_win..=call.end_win {
            if detected_raw[w] {
                energy_sum += spectrogram[w][band_lo..=band_hi].iter().sum::<f32>() / n_band;
                n_det += 1;
            }
        }
        if n_det == 0 { continue; }
        let det_energy  = energy_sum / n_det as f32;
        let search_wins = (SEARCH_SECS * sample_rate / WINDOW_SIZE as f32) as usize;
        let lo_win      = call.start_win.saturating_sub(search_wins);
        let hi_win      = (call.end_win + search_wins).min(n_windows - 1);

        let n_extra = detection::targeted_pulse_count(
            &spectrogram, lo_win, hi_win,
            call.start_win, call.end_win,
            peak_hz, hz_per_bin, SEARCH_BAND_HZ, det_energy, LOCAL_SEARCH_THRESH,
        );

        let search_t0 = pass_s - SEARCH_SECS;
        let search_t1 = pass_e + SEARCH_SECS;
        let other_bat_nearby = passes.iter().enumerate().any(|(j, p)| {
            j != i && !p.dubious && p.species != pass_spe
                && p.start_sec < search_t1 && p.end_sec > search_t0
        });
        passes[i].n_extra = if other_bat_nearby { 0 } else { n_extra };
    }

    // ── Per-pass energy ───────────────────────────────────────────────────────
    for pass in &mut passes {
        let win_start = (pass.start_sec * sample_rate / WINDOW_SIZE as f32) as usize;
        let win_end = ((pass.end_sec * sample_rate / WINDOW_SIZE as f32) as usize)
            .min(n_windows - 1);
        let mut energy_sum = 0.0f32;
        let mut peak_energy = 0.0f32;
        let mut n_det = 0usize;
        for w in win_start..=win_end {
            if detected_raw[w] {
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

    // ── dB normalisation → spec_bytes ────────────────────────────────────────
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
                if p <= 0.0 || max_power <= 0.0 { return 0u8; }
                let db = 10.0 * (p / max_power).log10();
                ((db - noise_floor_db) / (-noise_floor_db) * 255.0).clamp(0.0, 255.0) as u8
            })
        })
        .collect();

    // ── Subsample spectrogram columns for display ─────────────────────────────
    // Cap at MAX_DISPLAY_WINDOWS so the HTML file stays manageable (<~3 MB of
    // base64) regardless of recording length.  call boundaries are converted to
    // display-window indices; passes remain in seconds so are unaffected.
    const MAX_DISPLAY_WINDOWS: usize = 4_000;
    let stride = (n_windows / MAX_DISPLAY_WINDOWS).max(1);
    let display_n_windows = (n_windows + stride - 1) / stride;

    let display_spec_bytes: Vec<u8>;
    let display_detected: Vec<bool>;
    let display_calls: Vec<CallGroupInfo>;
    let display_window_size: usize;

    if stride == 1 {
        display_spec_bytes = spec_bytes;
        display_detected   = grouped_detected;
        display_calls      = calls;
        display_window_size = WINDOW_SIZE;
    } else {
        // Average spec_bytes across each stride block.
        let mut dsb = vec![0u8; display_n_windows * freq_bins];
        for dw in 0..display_n_windows {
            let w0  = dw * stride;
            let w1  = (w0 + stride).min(n_windows);
            let cnt = (w1 - w0) as u32;
            for b in 0..freq_bins {
                let sum: u32 = (w0..w1)
                    .map(|w| spec_bytes[w * freq_bins + b] as u32)
                    .sum();
                dsb[dw * freq_bins + b] = (sum / cnt) as u8;
            }
        }
        // OR detected over each stride block.
        let ddet: Vec<bool> = (0..display_n_windows)
            .map(|dw| {
                let w0 = dw * stride;
                let w1 = (w0 + stride).min(n_windows);
                grouped_detected[w0..w1].iter().any(|&d| d)
            })
            .collect();
        // Remap call group boundaries to display-window indices.
        let dcalls: Vec<CallGroupInfo> = calls
            .iter()
            .map(|c| CallGroupInfo {
                start_win: c.start_win / stride,
                end_win:   c.end_win   / stride,
                start_sec: c.start_sec,
                end_sec:   c.end_sec,
                peaks:     c.peaks.clone(),
            })
            .collect();
        display_spec_bytes  = dsb;
        display_detected    = ddet;
        display_calls       = dcalls;
        display_window_size = WINDOW_SIZE * stride;
    }

    // ── Assemble result ───────────────────────────────────────────────────────
    let duration_sec = n_windows as f32 * WINDOW_SIZE as f32 / sample_rate;
    let pass_records: Vec<PassRecord> = passes
        .iter()
        .enumerate()
        .map(|(i, p)| PassRecord::from_pass(i, p))
        .collect();

    Ok(RecordingResult {
        file_name,
        sample_rate: spec.sample_rate,
        duration_sec,
        passes: pass_records,
        stem,
        n_windows:   display_n_windows,
        freq_bins,
        hz_per_bin,
        window_size: display_window_size,
        spec_bytes:  display_spec_bytes,
        detected:    display_detected,
        calls:       display_calls,
        raw_passes:  passes,
    })
}
