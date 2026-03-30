use rustfft::{num_complex::Complex, FftPlanner};

/// FFT-process `samples` into non-overlapping Hann-windowed frames.
/// Returns one power spectrum (`Vec<f32>` of length `window_size/2`) per complete frame.
pub fn compute_spectrogram(samples: &[f32], window_size: usize) -> Vec<Vec<f32>> {
    let freq_bins = window_size / 2;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(window_size);

    samples
        .chunks(window_size)
        .filter(|c| c.len() == window_size)
        .map(|chunk| {
            let mut buffer: Vec<Complex<f32>> = chunk
                .iter()
                .enumerate()
                .map(|(n, &s)| {
                    let hann = 0.5
                        * (1.0
                            - (2.0 * std::f32::consts::PI * n as f32
                                / (window_size - 1) as f32)
                                .cos());
                    Complex { re: s * hann, im: 0.0 }
                })
                .collect();
            fft.process(&mut buffer);
            buffer[..freq_bins].iter().map(|c| c.norm_sqr()).collect()
        })
        .collect()
}

/// Default minimum ratio of bat-band mean energy to whole-spectrum mean energy.
///
/// This secondary check complements the adaptive noise-floor test.  Broadband
/// noise sources (traffic, wind) elevate all frequency bins roughly equally,
/// keeping this ratio near 1.0.  Real bat calls concentrate energy in the bat
/// band and push the ratio well above 1.  Setting the minimum to 1.05 rejects
/// broadband interference while passing genuine bat activity.
pub const SPECTRAL_RATIO_MIN_DEFAULT: f32 = 1.05;

/// Flag windows that contain bat energy using a two-condition detector.
///
/// **Condition 1 — adaptive noise floor (handles high-frequency insect noise)**
///
/// For each window the mean bat-band energy is compared against a local noise
/// floor estimate derived from the surrounding `±noise_half_window` windows.
/// The noise floor is the **10th percentile** of bat-band energies in that
/// neighbourhood: because bats are present in a small fraction of frames even
/// during active surveys, the low percentile stays close to the true background
/// regardless of how loud or sustained the calls are.
///
/// `bat_band_energy  >  noise_floor_10th_pct  ×  threshold_factor`
///
/// **Condition 2 — spectral ratio (handles broadband low-frequency noise)**
///
/// The bat-band mean must also exceed the whole-spectrum mean by at least
/// `SPECTRAL_RATIO_MIN` (1.05).  Broadband noise (traffic, wind) raises all
/// bins equally so its ratio stays near 1.0 and the window is rejected even
/// when it passes Condition 1.  Real bat calls concentrate energy in the bat
/// band and clear this bar comfortably.
///
/// Both conditions must be satisfied for a window to be flagged.
pub fn detect_bat_windows(
    spectrogram: &[Vec<f32>],
    bin_low: usize,
    bin_high: usize,
    threshold_factor: f32,
    noise_half_window: usize,
    spectral_ratio_min: f32,
) -> Vec<bool> {
    let n = spectrogram.len();
    if n == 0 {
        return vec![];
    }

    // Step 1 — peak bat-band energy per window.
    //
    // Using the per-window MAX rather than the mean gives ~14 dB better sensitivity.
    // A bat call concentrates energy in roughly 10 bins out of ~268 in the bat band;
    // averaging over all 268 dilutes the signal by a factor of ~27.  The max directly
    // reflects the strongest bin and is not diluted.  The adaptive noise floor (p10 of
    // neighbourhood maxes) adapts to the typical peak background level, so a consistent
    // electronic tone or hot pixel does not cause false positives — it simply raises the
    // local floor just as a broadband noise source would with the old mean approach.
    let bat_energies: Vec<f32> = spectrogram
        .iter()
        .map(|w| {
            w[bin_low..=bin_high]
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect();

    let n_bat_bins = (bin_high - bin_low + 1) as f32;

    // Step 2 — per-window detection: adaptive floor AND spectral ratio.
    bat_energies
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            // Condition 1: adaptive noise-floor check (max-based).
            let lo = i.saturating_sub(noise_half_window);
            let hi = (i + noise_half_window + 1).min(n);
            let mut buf: Vec<f32> = bat_energies[lo..hi].to_vec();
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p10 = buf[(buf.len() - 1) / 10];
            let noise_floor = p10.max(1e-12);
            let adaptive_ok = e > noise_floor * threshold_factor;

            // Condition 2: spectral-ratio check (mean-based for both sides).
            // The ratio must use means, not the max: if we used the bat-band max here,
            // broadband noise would appear to have a high ratio (the max of ~268 noise
            // bins is several times the mean of ~240 non-bat bins) and would pass
            // incorrectly.  With means, broadband noise gives ratio ≈ 1 and is rejected.
            let w = &spectrogram[i];
            let e_mean = w[bin_low..=bin_high].iter().sum::<f32>() / n_bat_bins;
            let n_below = bin_low.saturating_sub(1);           // bins 1..bin_low
            let n_above = w.len().saturating_sub(bin_high + 1); // bins bin_high+1..
            let n_non_bat = (n_below + n_above) as f32;
            let non_bat_mean = if n_non_bat > 0.0 {
                let below: f32 = w[1..bin_low].iter().sum();
                let above: f32 = w[(bin_high + 1)..].iter().sum();
                (below + above) / n_non_bat
            } else {
                // Bat band covers the whole spectrum; fall back to whole-spectrum mean.
                w[1..].iter().sum::<f32>() / (w.len().saturating_sub(1)) as f32
            };
            // Primary ratio check: bat-band mean vs non-bat mean.
            let standard_ratio_ok = non_bat_mean > 0.0
                && e_mean / non_bat_mean > spectral_ratio_min;

            // Horseshoe-bat rescue: compare the top 40 % of the bat band
            // (78–120 kHz for a typical 15–120 kHz band) against the lower 60 %.
            // A narrow CF signal at 80–120 kHz concentrates nearly all its energy
            // in 2–4 bins at the top of the band, giving a top/bottom ratio of
            // 50–500×.  FM bat calls and broadband noise with energy below 78 kHz
            // leave the top portion at noise level, so their ratio stays near 1.
            // This is completely independent of the non-bat-band comparison, so it
            // works even when there is no signal below 15 kHz or above 120 kHz.
            let top_lo = bin_low + (bin_high - bin_low) * 3 / 5; // boundary at 60 %
            let n_top    = (bin_high + 1).saturating_sub(top_lo) as f32;
            let n_bottom = top_lo.saturating_sub(bin_low) as f32;
            let top_mean = if n_top > 0.0 {
                w[top_lo..=bin_high].iter().sum::<f32>() / n_top
            } else { 0.0 };
            let bottom_mean = if n_bottom > 0.0 {
                w[bin_low..top_lo].iter().sum::<f32>() / n_bottom
            } else { f32::INFINITY }; // no bottom bins → ratio near 0, guard fails
            let horseshoe_ok = bottom_mean > 0.0 && top_mean / bottom_mean > 5.0;

            let ratio_ok = standard_ratio_ok || horseshoe_ok;

            adaptive_ok && ratio_ok
        })
        .collect()
}

/// Per-window diagnostic metrics written when `--diagnose` is active.
pub struct WindowDiag {
    pub time_s: f32,
    /// Max energy in bat band (condition 1 signal).
    pub bat_max: f32,
    /// Adaptive noise floor (10th-percentile of neighbourhood maxes).
    pub noise_floor: f32,
    /// bat_max / noise_floor (condition 1 ratio; must exceed threshold_factor).
    pub cond1_ratio: f32,
    /// Mean energy in bat band (condition 2 signal).
    pub bat_mean: f32,
    /// Mean energy outside bat band (condition 2 denominator).
    pub nonbat_mean: f32,
    /// bat_mean / nonbat_mean (condition 2 ratio; must exceed spectral_ratio_min).
    pub cond2_ratio: f32,
    pub cond1_pass: bool,
    pub cond2_pass: bool,
    pub detected: bool,
}

/// Same calculation as `detect_bat_windows` but returns per-window diagnostics
/// instead of (or as well as) the detection booleans.
pub fn detect_bat_windows_diag(
    spectrogram: &[Vec<f32>],
    bin_low: usize,
    bin_high: usize,
    threshold_factor: f32,
    noise_half_window: usize,
    spectral_ratio_min: f32,
    sample_rate: f32,
    window_size: usize,
) -> Vec<WindowDiag> {
    let n = spectrogram.len();
    if n == 0 {
        return vec![];
    }

    let bat_energies: Vec<f32> = spectrogram
        .iter()
        .map(|w| {
            w[bin_low..=bin_high]
                .iter()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect();

    let n_bat_bins = (bin_high - bin_low + 1) as f32;

    bat_energies
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            let lo = i.saturating_sub(noise_half_window);
            let hi = (i + noise_half_window + 1).min(n);
            let mut buf: Vec<f32> = bat_energies[lo..hi].to_vec();
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p10 = buf[(buf.len() - 1) / 10];
            let noise_floor = p10.max(1e-12);
            let cond1_ratio = e / noise_floor;
            let cond1_pass = cond1_ratio > threshold_factor;

            let w = &spectrogram[i];
            let bat_mean = w[bin_low..=bin_high].iter().sum::<f32>() / n_bat_bins;
            let n_below = bin_low.saturating_sub(1);
            let n_above = w.len().saturating_sub(bin_high + 1);
            let n_non_bat = (n_below + n_above) as f32;
            let nonbat_mean = if n_non_bat > 0.0 {
                let below: f32 = w[1..bin_low].iter().sum();
                let above: f32 = w[(bin_high + 1)..].iter().sum();
                (below + above) / n_non_bat
            } else {
                w[1..].iter().sum::<f32>() / (w.len().saturating_sub(1)) as f32
            };
            let cond2_ratio = if nonbat_mean > 0.0 { bat_mean / nonbat_mean } else { 0.0 };
            let standard_cond2 = nonbat_mean > 0.0 && cond2_ratio > spectral_ratio_min;
            // Horseshoe-bat rescue: mirrors the logic in detect_bat_windows.
            let top_lo = bin_low + (bin_high - bin_low) * 3 / 5;
            let n_top    = (bin_high + 1).saturating_sub(top_lo) as f32;
            let n_bottom = top_lo.saturating_sub(bin_low) as f32;
            let top_mean = if n_top > 0.0 {
                w[top_lo..=bin_high].iter().sum::<f32>() / n_top
            } else { 0.0 };
            let bottom_mean_hf = if n_bottom > 0.0 {
                w[bin_low..top_lo].iter().sum::<f32>() / n_bottom
            } else { f32::INFINITY };
            let horseshoe_ok = bottom_mean_hf > 0.0 && top_mean / bottom_mean_hf > 5.0;
            let cond2_pass = standard_cond2 || horseshoe_ok;

            WindowDiag {
                time_s: i as f32 * window_size as f32 / sample_rate,
                bat_max: e,
                noise_floor,
                cond1_ratio,
                bat_mean,
                nonbat_mean,
                cond2_ratio,
                cond1_pass,
                cond2_pass,
                detected: cond1_pass && cond2_pass,
            }
        })
        .collect()
}

/// Group consecutive bat-detected windows into call groups, merging gaps ≤ `gap_fill`.
///
/// `detected[i]` is `true` when window `i` contains bat energy.
pub fn group_calls(detected: &[bool], gap_fill: usize) -> Vec<(usize, usize)> {
    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &is_bat) in detected.iter().enumerate() {
        match (is_bat, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                raw.push((s, i - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((s, detected.len() - 1));
    }

    let mut merged: Vec<(usize, usize)> = Vec::new();
    for group in raw {
        if let Some(last) = merged.last_mut()
            && group.0 <= last.1 + gap_fill + 1
        {
            last.1 = group.1;
            continue;
        }
        merged.push(group);
    }
    merged
}

/// Count distinct pulses in a narrow frequency band around `peak_hz` within
/// windows `lo_win..=hi_win`, excluding the original group `exclude_lo..=exclude_hi`.
///
/// A "pulse" is a run of consecutive windows whose mean band energy exceeds
/// `detected_energy * rel_thresh`.  Calibrating to `detected_energy` makes the
/// search self-scaling: if the original pulse was weak the bar is low; if it was
/// loud nearby calls must also be loud to count.
#[allow(clippy::too_many_arguments)]
pub fn targeted_pulse_count(
    spectrogram: &[Vec<f32>],
    lo_win: usize,
    hi_win: usize,
    exclude_lo: usize,
    exclude_hi: usize,
    peak_hz: f32,
    hz_per_bin: f32,
    band_hz: f32,
    detected_energy: f32,
    rel_thresh: f32,
) -> usize {
    if spectrogram.is_empty() || detected_energy <= 0.0 { return 0; }
    let n_bins = spectrogram[0].len();
    let band_lo = ((peak_hz - band_hz).max(0.0) / hz_per_bin) as usize;
    let band_hi = (((peak_hz + band_hz) / hz_per_bin).round() as usize).min(n_bins.saturating_sub(1));
    if band_lo > band_hi { return 0; }
    let n_band = (band_hi - band_lo + 1) as f32;
    let threshold = detected_energy * rel_thresh;

    let mut n_pulses = 0usize;
    let mut in_pulse = false;

    for (w, spec_win) in spectrogram.iter().enumerate().take(hi_win + 1).skip(lo_win) {
        if w >= exclude_lo && w <= exclude_hi {
            in_pulse = false;
            continue;
        }
        let energy = spec_win[band_lo..=band_hi].iter().sum::<f32>() / n_band;
        if energy > threshold {
            if !in_pulse {
                n_pulses += 1;
                in_pulse = true;
            }
        } else {
            in_pulse = false;
        }
    }
    n_pulses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_calls_empty() {
        assert_eq!(group_calls(&[], 2), vec![]);
    }

    #[test]
    fn group_calls_no_bats() {
        assert_eq!(group_calls(&[false, false, false], 2), vec![]);
    }

    #[test]
    fn group_calls_all_bats() {
        assert_eq!(group_calls(&[true, true, true], 0), vec![(0, 2)]);
    }

    #[test]
    fn group_calls_single_window() {
        assert_eq!(group_calls(&[false, true, false], 0), vec![(1, 1)]);
    }

    #[test]
    fn group_calls_two_groups_no_merge() {
        // Gap of 2 windows, gap_fill = 1 → should NOT merge
        let det = [true, true, false, false, true, true];
        assert_eq!(group_calls(&det, 1), vec![(0, 1), (4, 5)]);
    }

    #[test]
    fn group_calls_two_groups_merge() {
        // Gap of 2 windows, gap_fill = 2 → should merge into one group
        let det = [true, true, false, false, true, true];
        assert_eq!(group_calls(&det, 2), vec![(0, 5)]);
    }

    #[test]
    fn group_calls_ends_on_bat() {
        // Last window is bat → group should run to end
        let det = [false, true, true];
        assert_eq!(group_calls(&det, 0), vec![(1, 2)]);
    }
}
