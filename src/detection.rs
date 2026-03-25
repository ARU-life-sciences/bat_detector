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

/// Minimum ratio of bat-band mean energy to whole-spectrum mean energy.
///
/// This secondary check complements the adaptive noise-floor test.  Broadband
/// noise sources (traffic, wind) elevate all frequency bins roughly equally,
/// keeping this ratio near 1.0.  Real bat calls concentrate energy in the bat
/// band and push the ratio well above 1.  Setting the minimum to 1.05 rejects
/// broadband interference while passing genuine bat activity.
const SPECTRAL_RATIO_MIN: f32 = 1.05;

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
) -> Vec<bool> {
    let n = spectrogram.len();
    if n == 0 {
        return vec![];
    }

    // Step 1 — mean bat-band energy per window.
    let n_bat_bins = (bin_high - bin_low + 1) as f32;
    let bat_energies: Vec<f32> = spectrogram
        .iter()
        .map(|w| w[bin_low..=bin_high].iter().sum::<f32>() / n_bat_bins)
        .collect();

    // Step 2 — per-window detection: adaptive floor AND spectral ratio.
    bat_energies
        .iter()
        .enumerate()
        .map(|(i, &e)| {
            // Condition 1: adaptive noise-floor check.
            let lo = i.saturating_sub(noise_half_window);
            let hi = (i + noise_half_window + 1).min(n);
            let mut buf: Vec<f32> = bat_energies[lo..hi].to_vec();
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p10 = buf[(buf.len() - 1) / 10];
            let noise_floor = p10.max(1e-12);
            let adaptive_ok = e > noise_floor * threshold_factor;

            // Condition 2: spectral-ratio check.
            let w = &spectrogram[i];
            let n_all = (w.len().saturating_sub(1)) as f32;
            let total_mean = if n_all > 0.0 {
                w[1..].iter().sum::<f32>() / n_all
            } else {
                0.0
            };
            let ratio_ok = total_mean > 0.0 && e / total_mean > SPECTRAL_RATIO_MIN;

            adaptive_ok && ratio_ok
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
        if let Some(last) = merged.last_mut() {
            if group.0 <= last.1 + gap_fill + 1 {
                last.1 = group.1;
                continue;
            }
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

    for w in lo_win..=hi_win {
        if w >= exclude_lo && w <= exclude_hi {
            in_pulse = false;
            continue;
        }
        let energy = spectrogram[w][band_lo..=band_hi].iter().sum::<f32>() / n_band;
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
