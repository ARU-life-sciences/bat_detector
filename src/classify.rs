use crate::features::CallFeatures;

/// British bat key (Cornes, Bedfordshire Bat Group, 2008).
/// Returns `(species name, diagnostic notes)`.
pub fn classify_british(f: &CallFeatures) -> (&'static str, &'static str) {
    // ── Steps 1–2: Horseshoe bats (narrowband CF) ─────────────────────────────
    if f.is_cf {
        return match f.peak_hz as u32 {
            78_000..=87_000 => (
                "Greater Horseshoe Bat (Rhinolophus ferrumequinum)",
                "CF ~83 kHz; narrowband; prolonged whistle up to 50 ms",
            ),
            104_000..=116_000 => (
                "Lesser Horseshoe Bat (Rhinolophus hipposideros)",
                "CF ~110 kHz; narrowband; highly directional call",
            ),
            _ => (
                "Horseshoe bat sp. (unresolved)",
                "CF call confirmed but peak outside known British ranges",
            ),
        };
    }

    // ── Step 3: FM+CF "slap" vs pure FM "click" ───────────────────────────────
    // Pipistrelles and big bats end their FM sweep with a CF tail that
    // concentrates energy → high cf_tail_ratio ("slap").
    // Myotis and plecotines are pure FM with no CF tail → low cf_tail_ratio ("click").
    let has_cf_tail = f.cf_tail_ratio > 0.28;

    if has_cf_tail {
        // ── Step 4: Pipistrelles (peak > 35 kHz) vs big bats ─────────────────
        if f.peak_hz > 35_000.0 {
            // ── Step 5: Nathusius' vs Common / Soprano ────────────────────────
            if f.peak_hz < 40_000.0 || f.rep_rate < 8.0 {
                return (
                    "Nathusius' Pipistrelle (Pipistrellus nathusii)",
                    "Peak <40 kHz; slow rep ~6-7/s; FM+CF call",
                );
            }
            // ── Step 6: Soprano vs Common ─────────────────────────────────────
            if f.peak_hz >= 50_000.0 {
                return (
                    "Soprano Pipistrelle (Pipistrellus pygmaeus)",
                    "Peak typically 52-55 kHz; medium-rapid rep >=10/s",
                );
            }
            if f.peak_hz >= 43_000.0 {
                return (
                    "Common Pipistrelle (Pipistrellus pipistrellus)",
                    "Peak typically 43-46 kHz; medium-rapid rep >=10/s",
                );
            }
            return (
                "Common or Nathusius' Pipistrelle",
                "Peak near 40-43 kHz boundary; habitat context needed",
            );
        }

        // ── Steps 7–8: Big bats ───────────────────────────────────────────────
        if f.peak_hz >= 27_000.0 {
            return (
                "Serotine (Eptesicus serotinus)",
                "Peak 27-35 kHz; medium rep ~10/s; drunken syncopated rhythm; \
                 NB: may be confused with Barbastelle, Brown Long-eared or Greater Mouse-eared",
            );
        }
        if f.freq_low_hz <= 21_000.0 {
            return (
                "Noctule (Nyctalus noctula)",
                "Peak <=20 kHz in at least some calls; slow rep 3-6/s; chip-chop alternation",
            );
        }
        return (
            "Leisler's Bat (Nyctalus leisleri)",
            "All calls above 21 kHz; slow-medium rep; less marked alternation than Noctule",
        );
    }

    // ── Steps 9–13: Pure FM — Myotis, Barbastelle, long-eared ────────────────

    // Step 9: Barbastelle — narrow range 30–45 kHz, peak 32–34 kHz
    let freq_range = f.freq_high_hz - f.freq_low_hz;
    if (31_000.0..=36_000.0).contains(&f.peak_hz) && freq_range < 18_000.0 {
        return (
            "Barbastelle (Barbastella barbastellus)",
            "Peak 32-34 kHz; narrow range 30-45 kHz; tock quality; \
             rapid knocking rhythm; two alternating peaks ~33 & ~41 kHz visible on sonogram",
        );
    }

    // Step 11 (part 1): calls audible above 90 kHz → Natterer's or Bechstein's
    if f.freq_high_hz > 90_000.0 {
        return if f.freq_low_hz < 30_000.0 {
            (
                "Natterer's Bat (Myotis nattereri)",
                "Audible above 90 kHz and below 30 kHz; rapid rep",
            )
        } else {
            (
                "Bechstein's Bat (Myotis bechsteinii)",
                "Audible above 90 kHz but not below 30 kHz; medium rep 9-11/s",
            )
        };
    }

    // Step 11 (part 2): inaudible above ~65 kHz → long-eared bats
    if f.freq_high_hz < 65_000.0 {
        if (45_000.0..=55_000.0).contains(&f.peak_hz) {
            return (
                "Brown Long-eared Bat (Plecotus auritus)",
                "Peak 45-55 kHz; rapid rep; inaudible above 60 kHz; \
                 NB: may occasionally produce louder Serotine-like calls",
            );
        }
        if f.peak_hz < 40_000.0 {
            return (
                "Grey Long-eared Bat (Plecotus austriacus)",
                "Peak <40 kHz; medium rep; rare in Britain",
            );
        }
        return (
            "Long-eared bat sp. (Plecotus sp.)",
            "Inaudible above 65 kHz",
        );
    }

    // Step 10: broadband FM — general Myotis
    if f.rep_rate > 10.0 {
        return (
            "Myotis sp. — probably Daubenton's / Whiskered / Brandt's",
            "Rapid rep >10/s; broadband FM; \
             Daubenton's confirmed by low (<15 cm) flight over water",
        );
    }
    (
        "Myotis sp. (unresolved)",
        "Broadband FM; audible over wide range; visual confirmation needed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::CallFeatures;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn cf(peak_hz: f32) -> CallFeatures {
        CallFeatures {
            peak_hz,
            bandwidth_hz: 2_000.0,
            freq_low_hz: peak_hz - 1_000.0,
            freq_high_hz: peak_hz + 1_000.0,
            cf_tail_ratio: 0.9,
            rep_rate: 10.0,
            is_cf: true,
        }
    }

    /// FM+CF "slap" call (cf_tail_ratio > 0.28, is_cf = false).
    fn fm_cf(peak_hz: f32, freq_low_hz: f32, freq_high_hz: f32, rep_rate: f32) -> CallFeatures {
        CallFeatures {
            peak_hz,
            bandwidth_hz: 15_000.0,
            freq_low_hz,
            freq_high_hz,
            cf_tail_ratio: 0.5,
            rep_rate,
            is_cf: false,
        }
    }

    /// Pure FM "click" call (cf_tail_ratio < 0.28, is_cf = false).
    fn pure_fm(peak_hz: f32, freq_low_hz: f32, freq_high_hz: f32, rep_rate: f32) -> CallFeatures {
        let bw = freq_high_hz - freq_low_hz;
        CallFeatures {
            peak_hz,
            bandwidth_hz: bw,
            freq_low_hz,
            freq_high_hz,
            cf_tail_ratio: 0.1,
            rep_rate,
            is_cf: false,
        }
    }

    // ── CF paths (Steps 1–2) ──────────────────────────────────────────────────

    #[test]
    fn greater_horseshoe() {
        let (sp, _) = classify_british(&cf(83_000.0));
        assert!(sp.contains("Greater Horseshoe"), "{}", sp);
    }

    #[test]
    fn lesser_horseshoe() {
        let (sp, _) = classify_british(&cf(110_000.0));
        assert!(sp.contains("Lesser Horseshoe"), "{}", sp);
    }

    #[test]
    fn horseshoe_unresolved() {
        let (sp, _) = classify_british(&cf(60_000.0));
        assert!(sp.contains("unresolved"), "{}", sp);
    }

    // ── FM+CF paths (Steps 4–8) ───────────────────────────────────────────────

    #[test]
    fn soprano_pipistrelle() {
        let f = fm_cf(53_000.0, 40_000.0, 65_000.0, 12.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Soprano"), "{}", sp);
    }

    #[test]
    fn common_pipistrelle() {
        let f = fm_cf(45_000.0, 35_000.0, 60_000.0, 12.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Common Pipistrelle"), "{}", sp);
    }

    #[test]
    fn nathusius_low_peak() {
        // Peak < 40 kHz → Nathusius'
        let f = fm_cf(38_000.0, 28_000.0, 52_000.0, 7.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Nathusius"), "{}", sp);
    }

    #[test]
    fn nathusius_slow_rep() {
        // Peak in 40–50 kHz range but slow rep rate → Nathusius'
        let f = fm_cf(42_000.0, 30_000.0, 55_000.0, 6.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Nathusius"), "{}", sp);
    }

    #[test]
    fn serotine() {
        let f = fm_cf(30_000.0, 22_000.0, 45_000.0, 10.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Serotine"), "{}", sp);
    }

    #[test]
    fn noctule() {
        // freq_low ≤ 21 kHz → Noctule
        let f = fm_cf(20_000.0, 18_000.0, 32_000.0, 4.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Noctule"), "{}", sp);
    }

    #[test]
    fn leisleri() {
        // Peak < 27 kHz, freq_low > 21 kHz, has CF tail → Leisler's
        let f = fm_cf(25_000.0, 22_000.0, 35_000.0, 5.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Leisler"), "{}", sp);
    }

    // ── Pure FM paths (Steps 9–13) ────────────────────────────────────────────

    #[test]
    fn barbastelle() {
        // Peak 31–36 kHz, narrow range < 18 kHz
        let f = pure_fm(33_000.0, 28_000.0, 43_000.0, 10.0); // range = 15 kHz
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Barbastelle"), "{}", sp);
    }

    #[test]
    fn natterers() {
        // Audible above 90 kHz and below 30 kHz
        let f = pure_fm(50_000.0, 20_000.0, 95_000.0, 12.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Natterer"), "{}", sp);
    }

    #[test]
    fn bechsteins() {
        // Audible above 90 kHz but NOT below 30 kHz
        let f = pure_fm(55_000.0, 35_000.0, 95_000.0, 10.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Bechstein"), "{}", sp);
    }

    #[test]
    fn brown_long_eared() {
        // freq_high < 65 kHz, peak 45–55 kHz
        let f = pure_fm(50_000.0, 35_000.0, 62_000.0, 12.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Brown Long-eared"), "{}", sp);
    }

    #[test]
    fn grey_long_eared() {
        // freq_high < 65 kHz, peak < 40 kHz
        let f = pure_fm(35_000.0, 25_000.0, 60_000.0, 8.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Grey Long-eared"), "{}", sp);
    }

    #[test]
    fn myotis_rapid() {
        // Broadband FM, rep > 10/s → probably Daubenton's etc.
        let f = pure_fm(45_000.0, 25_000.0, 80_000.0, 15.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("Daubenton"), "{}", sp);
    }

    #[test]
    fn myotis_unresolved() {
        // Broadband FM, rep ≤ 10/s → unresolved Myotis
        let f = pure_fm(45_000.0, 25_000.0, 80_000.0, 8.0);
        let (sp, _) = classify_british(&f);
        assert!(sp.contains("unresolved"), "{}", sp);
    }
}
