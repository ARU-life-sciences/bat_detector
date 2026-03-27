#[derive(Clone)]
pub struct CallFeatures {
    pub peak_hz: f32,
    pub bandwidth_hz: f32, // −10 dB
    pub freq_low_hz: f32,  // −20 dB lower bound (from 18 kHz)
    pub freq_high_hz: f32, // −20 dB upper bound
    pub cf_tail_ratio: f32,
    pub rep_rate: f32,
    pub is_cf: bool,
    /// Mean individual call duration in milliseconds.
    /// Estimated as (detected windows in group) / (pulses in group) × window_ms.
    pub mean_call_duration_ms: f32,
    /// Number of individual pulses counted within this call group.
    pub n_pulses: usize,
}

/// Find significant spectral peaks in `spectrum[bin_low..=bin_high]`.
///
/// Returns bin indices sorted by ascending frequency.
/// Only peaks above `min_rel_height × global_max` and separated by at least
/// `min_sep_hz` Hz from each other are returned.
fn find_peaks(
    spectrum: &[f32],
    bin_low: usize,
    bin_high: usize,
    hz_per_bin: f32,
    min_sep_hz: f32,
    min_rel_height: f32,
) -> Vec<usize> {
    let min_sep = (min_sep_hz / hz_per_bin).round() as usize;

    let (global_rel, &global_max) = spectrum[bin_low..=bin_high]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    let global_peak = bin_low + global_rel;
    let threshold = global_max * min_rel_height;

    // Collect all local maxima above threshold (global peak always included)
    let mut candidates: Vec<(usize, f32)> = vec![(global_peak, global_max)];
    for i in (bin_low + 1)..bin_high {
        if i != global_peak
            && spectrum[i] >= threshold
            && spectrum[i] > spectrum[i - 1]
            && spectrum[i] > spectrum[i + 1]
        {
            candidates.push((i, spectrum[i]));
        }
    }

    // Greedy selection by descending power, enforcing minimum separation
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let mut selected: Vec<usize> = Vec::new();
    for (bin, _) in candidates {
        if selected
            .iter()
            .all(|&s| (bin as isize - s as isize).unsigned_abs() >= min_sep)
        {
            selected.push(bin);
        }
    }
    selected.sort_unstable();
    selected
}

/// Extract `CallFeatures` for one spectral peak.
///
/// `left_bound`/`right_bound` constrain bandwidth and frequency-range searches
/// to this peak's territory (midpoints between adjacent peaks), preventing
/// measurements from bleeding into a neighbouring species' signal.
#[allow(clippy::too_many_arguments)]
fn features_for_peak(
    mean_power: &[f32],
    spectrogram: &[Vec<f32>],
    start: usize,
    end: usize,
    n_detected: usize,
    peak_bin: usize,
    bin_id_low: usize,
    left_bound: usize,
    right_bound: usize,
    hz_per_bin: f32,
    sample_rate: f32,
    window_size: usize,
) -> CallFeatures {
    let peak_power = mean_power[peak_bin];
    let peak_hz = peak_bin as f32 * hz_per_bin;

    // −10 dB bandwidth, clamped to this peak's territory
    let thresh_10db = peak_power / 10.0;
    let bw_low = mean_power[left_bound..=peak_bin]
        .iter()
        .rposition(|&p| p < thresh_10db)
        .map(|i| left_bound + i + 1)
        .unwrap_or(left_bound);
    let bw_high = mean_power[peak_bin..=right_bound]
        .iter()
        .position(|&p| p < thresh_10db)
        .map(|i| peak_bin + i)
        .unwrap_or(right_bound);
    let bandwidth_hz = (bw_high - bw_low) as f32 * hz_per_bin;

    // −20 dB frequency range (extended lower bound captures Noctule calls ~18 kHz)
    let thresh_20db = peak_power / 100.0;
    let freq_low_bin = mean_power[bin_id_low..=peak_bin]
        .iter()
        .rposition(|&p| p < thresh_20db)
        .map(|i| bin_id_low + i + 1)
        .unwrap_or(bin_id_low);
    let freq_high_bin = mean_power[peak_bin..=right_bound]
        .iter()
        .position(|&p| p < thresh_20db)
        .map(|i| peak_bin + i)
        .unwrap_or(right_bound);
    let freq_low_hz = freq_low_bin as f32 * hz_per_bin;
    let freq_high_hz = freq_high_bin as f32 * hz_per_bin;

    // CF-tail energy concentration: ±4 bins around peak vs total territory energy
    let narrow_lo = peak_bin.saturating_sub(4).max(left_bound);
    let narrow_hi = (peak_bin + 4).min(right_bound);
    let narrow_energy: f32 = mean_power[narrow_lo..=narrow_hi].iter().sum();
    let band_energy: f32 = mean_power[left_bound..=right_bound].iter().sum();
    let cf_tail_ratio = narrow_energy / band_energy.max(1e-30);

    // Repetition rate: count energy peaks in a narrow band around *this* peak's
    // frequency — isolates this species' pulse rate even when two species overlap.
    let rep_lo = peak_bin.saturating_sub(6).max(bin_id_low);
    let rep_hi = (peak_bin + 6).min(right_bound);
    let min_sep_idx = ((sample_rate / window_size as f32) * 0.025).max(3.0) as usize;
    let energies: Vec<f32> = (start..=end)
        .map(|w| spectrogram[w][rep_lo..=rep_hi].iter().sum::<f32>())
        .collect();
    let mut n_pulses = 0usize;
    let mut last_peak_idx = 0usize;
    for i in 1..energies.len().saturating_sub(1) {
        if energies[i] > energies[i - 1]
            && energies[i] > energies[i + 1]
            && (n_pulses == 0 || i >= last_peak_idx + min_sep_idx)
        {
            n_pulses += 1;
            last_peak_idx = i;
        }
    }
    // Always credit at least one pulse — the group exists because something was detected.
    if n_pulses == 0 { n_pulses = 1; }
    let duration_sec = (end - start + 1) as f32 * window_size as f32 / sample_rate;
    let rep_rate = n_pulses as f32 / duration_sec;

    // Mean individual call duration: detected "on" time divided by pulse count.
    // n_detected counts only windows that passed the detector (not gap-fill frames),
    // so this estimates the mean time the bat was actively emitting per pulse.
    let mean_call_duration_ms =
        n_detected as f32 / n_pulses as f32 * window_size as f32 / sample_rate * 1000.0;

    CallFeatures {
        peak_hz,
        bandwidth_hz,
        freq_low_hz,
        freq_high_hz,
        cf_tail_ratio,
        rep_rate,
        // True CF calls are narrowband, highly concentrated at the peak, AND in the
        // horseshoe-bat frequency range (≥70 kHz for British species).  Pipistrelle
        // CF tails fall at ~50–60 kHz and must not be flagged as CF calls.
        is_cf: (freq_high_hz - freq_low_hz) < 6_000.0
            && cf_tail_ratio > 0.7
            && peak_hz >= 70_000.0,
        mean_call_duration_ms,
        n_pulses,
    }
}

/// Extract features for every significant spectral peak in a call group.
///
/// Returns one `CallFeatures` per detected peak. Typically one, but returns
/// two or more when multiple species are calling simultaneously (each peak
/// separated by ≥ 10 kHz and ≥ 25% of the dominant peak's energy).
#[allow(clippy::too_many_arguments)]
pub fn extract_call_features(
    spectrogram: &[Vec<f32>],
    detected: &[bool],
    start: usize,
    end: usize,
    bin_id_low: usize,
    bin_low: usize,
    bin_high: usize,
    freq_bins: usize,
    hz_per_bin: f32,
    sample_rate: f32,
    window_size: usize,
) -> Vec<CallFeatures> {
    // Average only over windows actually detected as bat within this group,
    // so silence frames don't dilute the spectral features.
    let mut mean_power = vec![0.0f32; freq_bins];
    let mut n = 0usize;
    for (det_flag, spec_win) in detected[start..=end].iter().zip(spectrogram[start..=end].iter()) {
        if *det_flag {
            for (b, &p) in spec_win.iter().enumerate() {
                mean_power[b] += p;
            }
            n += 1;
        }
    }
    // Fallback: group exists but no individual windows are flagged (shouldn't happen).
    if n == 0 {
        n = end - start + 1;
        for spec_win in spectrogram[start..=end].iter() {
            for (b, &p) in spec_win.iter().enumerate() {
                mean_power[b] += p;
            }
        }
    }
    for p in &mut mean_power {
        *p /= n as f32;
    }

    let peak_bins = find_peaks(&mean_power, bin_low, bin_high, hz_per_bin, 10_000.0, 0.25);

    peak_bins
        .iter()
        .enumerate()
        .map(|(idx, &peak_bin)| {
            // Constrain each peak's measurements to the territory between its
            // neighbours (midpoint boundaries) so features don't bleed across.
            let left_bound =
                if idx == 0 { bin_low } else { (peak_bins[idx - 1] + peak_bin) / 2 };
            let right_bound = if idx + 1 == peak_bins.len() {
                bin_high
            } else {
                (peak_bin + peak_bins[idx + 1]) / 2
            };

            features_for_peak(
                &mean_power,
                spectrogram,
                start,
                end,
                n,
                peak_bin,
                bin_id_low,
                left_bound,
                right_bound,
                hz_per_bin,
                sample_rate,
                window_size,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a flat spectrogram (`n_windows` identical frames) with a single
    /// Gaussian peak centred at `peak_bin`.  All bins outside the peak are set
    /// to a small noise floor so the peak is clearly dominant.
    fn synthetic_spec(freq_bins: usize, peak_bin: usize, n_windows: usize) -> Vec<Vec<f32>> {
        let frame: Vec<f32> = (0..freq_bins)
            .map(|b| {
                let dist = (b as f32 - peak_bin as f32).abs();
                if dist == 0.0 {
                    1.0
                } else {
                    (-(dist * dist) / 8.0).exp().max(1e-6)
                }
            })
            .collect();
        vec![frame; n_windows]
    }

    // Parameters matching main.rs defaults for a 384 kHz recording.
    const SR: f32 = 384_000.0;
    const WS: usize = 1024;
    const HZ_PER_BIN: f32 = SR / WS as f32; // 375 Hz/bin
    const FREQ_BINS: usize = WS / 2;        // 512
    const BIN_LOW: usize = 53;              // ~20 kHz
    const BIN_HIGH: usize = 320;            // ~120 kHz
    const BIN_ID_LOW: usize = 48;           // ~18 kHz

    fn all_detected(n: usize) -> Vec<bool> {
        vec![true; n]
    }

    #[test]
    fn single_peak_frequency_is_correct() {
        // Soprano Pipistrelle territory: ~53 kHz → bin 141
        let peak_bin = (53_000.0 / HZ_PER_BIN).round() as usize; // 141
        let spec = synthetic_spec(FREQ_BINS, peak_bin, 20);
        let det = all_detected(20);

        let feats = extract_call_features(
            &spec, &det, 0, 19, BIN_ID_LOW, BIN_LOW, BIN_HIGH,
            FREQ_BINS, HZ_PER_BIN, SR, WS,
        );

        assert_eq!(feats.len(), 1, "expected exactly one peak");
        let f = &feats[0];
        let expected_hz = peak_bin as f32 * HZ_PER_BIN;
        assert!(
            (f.peak_hz - expected_hz).abs() < HZ_PER_BIN,
            "peak_hz {:.0} Hz is far from expected {:.0} Hz",
            f.peak_hz, expected_hz
        );
    }

    #[test]
    fn narrow_peak_is_detected_as_cf() {
        // A Gaussian with σ≈2 bins is very narrow (bandwidth ≪ 6 kHz)
        let peak_bin = (83_000.0 / HZ_PER_BIN).round() as usize;
        let spec = synthetic_spec(FREQ_BINS, peak_bin, 20);
        let det = all_detected(20);

        let feats = extract_call_features(
            &spec, &det, 0, 19, BIN_ID_LOW, BIN_LOW, BIN_HIGH,
            FREQ_BINS, HZ_PER_BIN, SR, WS,
        );

        assert_eq!(feats.len(), 1);
        assert!(feats[0].is_cf, "narrow peak should be classified as CF");
    }

    #[test]
    fn two_well_separated_peaks_detected() {
        // Place two equal peaks > 10 kHz apart so both pass find_peaks.
        let bin_a = (40_000.0 / HZ_PER_BIN).round() as usize;
        let bin_b = (80_000.0 / HZ_PER_BIN).round() as usize;
        let frame: Vec<f32> = (0..FREQ_BINS)
            .map(|b| {
                let da = (b as f32 - bin_a as f32).abs();
                let db = (b as f32 - bin_b as f32).abs();
                let pa = (-(da * da) / 8.0).exp();
                let pb = (-(db * db) / 8.0).exp();
                pa.max(pb).max(1e-6)
            })
            .collect();
        let spec = vec![frame; 20];
        let det = all_detected(20);

        let feats = extract_call_features(
            &spec, &det, 0, 19, BIN_ID_LOW, BIN_LOW, BIN_HIGH,
            FREQ_BINS, HZ_PER_BIN, SR, WS,
        );

        assert_eq!(feats.len(), 2, "expected two peaks for two well-separated signals");
        assert!(feats[0].peak_hz < feats[1].peak_hz, "peaks should be sorted by frequency");
    }

    #[test]
    fn cf_tail_ratio_high_for_narrow_peak() {
        // A narrow peak concentrates almost all energy within ±4 bins → high cf_tail_ratio.
        let peak_bin = (50_000.0 / HZ_PER_BIN).round() as usize;
        let spec = synthetic_spec(FREQ_BINS, peak_bin, 20);
        let det = all_detected(20);

        let feats = extract_call_features(
            &spec, &det, 0, 19, BIN_ID_LOW, BIN_LOW, BIN_HIGH,
            FREQ_BINS, HZ_PER_BIN, SR, WS,
        );

        assert_eq!(feats.len(), 1);
        assert!(
            feats[0].cf_tail_ratio > 0.8,
            "cf_tail_ratio {} should be high for a narrow peak",
            feats[0].cf_tail_ratio
        );
    }
}
