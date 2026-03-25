use std::io::Write as _;

use image::{ImageBuffer, Rgb};

use crate::features::CallFeatures;

// ── Colour map ────────────────────────────────────────────────────────────────

/// Maps t ∈ [0, 1] → heat colour: black → blue → cyan → green → yellow → red.
pub fn colormap(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    if t < 0.25 {
        let s = t * 4.0;
        [0, 0, (255.0 * s) as u8]
    } else if t < 0.5 {
        let s = (t - 0.25) * 4.0;
        [0, (255.0 * s) as u8, 255]
    } else if t < 0.75 {
        let s = (t - 0.5) * 4.0;
        [(255.0 * s) as u8, 255, (255.0 * (1.0 - s)) as u8]
    } else {
        let s = (t - 0.75) * 4.0;
        [255, (255.0 * (1.0 - s)) as u8, 0]
    }
}

// ── Output data structures ────────────────────────────────────────────────────

pub struct PeakInfo {
    pub features: CallFeatures,
    pub code: &'static str,
    pub species: &'static str,
    pub notes: &'static str,
}

/// One call group, potentially containing multiple simultaneous species.
pub struct CallGroupInfo {
    pub start_win: usize,
    pub end_win: usize,
    pub start_sec: f32,
    pub end_sec: f32,
    pub duration_ms: f32,
    pub peaks: Vec<PeakInfo>,
}

// ── Species-pass aggregation ──────────────────────────────────────────────────

/// One "pass" = all consecutive call groups of the same species within
/// `max_gap_sec` of each other.  When two species overlap in time they each
/// get their own PassInfo, giving one table row per species per pass.
pub struct PassInfo {
    pub species: &'static str,
    pub start_sec: f32,
    pub end_sec: f32,
    pub n_pulses: usize,
    /// Additional sub-threshold pulses found by the local search (single-pulse passes only).
    pub n_extra: usize,
    pub mean_peak_hz: f32,
    /// Standard deviation of peak frequency across pulses in this pass (Hz).
    /// Zero for single-pulse passes.  Low values indicate a tightly-clustered,
    /// species-consistent call sequence.
    pub peak_hz_std: f32,
    pub mean_freq_low_hz: f32,
    pub mean_freq_high_hz: f32,
    pub mean_bandwidth_hz: f32,
    pub mean_cf_tail_ratio: f32,
    pub mean_rep_rate: f32,
    pub is_cf: bool,
    /// Mean bat-band power (dB, linear FFT²) over detected windows — filled after construction.
    pub mean_energy_db: f32,
    /// Peak bat-band power (dB) across detected windows — filled after construction.
    pub peak_energy_db: f32,
    /// Six-letter species code (e.g. "PIPPYG").
    pub code: &'static str,
    /// Diagnostic notes from the classifier (same for all pulses of a given species).
    pub notes: &'static str,
    /// True when this single-pulse pass is entirely nested within a larger pass of a
    /// different species, making the identification unreliable.
    pub dubious: bool,
}

impl PassInfo {
    /// Identification confidence score in [0, 1].
    ///
    /// Combines two independent signals:
    ///
    /// **Pulse-count score** — rises from 0 towards 1 as more pulses accumulate.
    /// Formula: `1 − exp(−effective_n / 3)`, giving roughly:
    /// 0.28 (1 pulse) · 0.49 (2) · 0.63 (3) · 0.81 (5) · 0.97 (10+).
    /// For single-pulse passes the `n_extra` nearby sub-threshold pulses are
    /// added so that an isolated bat in an otherwise active area scores higher.
    ///
    /// **Frequency-consistency score** — how tightly clustered are the peak
    /// frequencies across pulses?  Uses the coefficient of variation (std/mean).
    /// A CV of 0 % gives 1.0; 10 % gives 0.5; 20 % gives 0.33.
    /// Single-pulse passes are given a consistency of 1.0 (nothing to measure).
    ///
    /// The final score is the product of the two components, clamped to [0, 1].
    /// Passes flagged as `dubious` always return 0.0.
    pub fn confidence(&self) -> f32 {
        if self.dubious {
            return 0.0;
        }
        let effective_n = if self.n_pulses == 1 {
            (1 + self.n_extra) as f32
        } else {
            self.n_pulses as f32
        };
        let pulse_score = 1.0 - (-effective_n / 3.0).exp();
        let consistency = if self.n_pulses > 1 && self.mean_peak_hz > 0.0 {
            let cv = self.peak_hz_std / self.mean_peak_hz;
            1.0 / (1.0 + 10.0 * cv)
        } else {
            1.0
        };
        (pulse_score * consistency).clamp(0.0, 1.0)
    }
}

// Per-call sample collected during pass accumulation.
struct PassSample {
    start: f32, end: f32,
    peak_hz: f32, freq_low_hz: f32, freq_high_hz: f32,
    bandwidth_hz: f32, cf_tail_ratio: f32, rep_rate: f32, is_cf: bool,
    code: &'static str,
    notes: &'static str,
}

/// Group call groups into species passes.  Calls of the same species separated
/// by ≤ `max_gap_sec` are merged; different species are always kept separate.
pub fn compute_passes(calls: &[CallGroupInfo], max_gap_sec: f32) -> Vec<PassInfo> {
    let mut by_species: std::collections::HashMap<&'static str, Vec<PassSample>> =
        std::collections::HashMap::new();

    for call in calls {
        for peak in &call.peaks {
            by_species.entry(peak.species).or_default().push(PassSample {
                start: call.start_sec,
                end: call.end_sec,
                peak_hz: peak.features.peak_hz,
                freq_low_hz: peak.features.freq_low_hz,
                freq_high_hz: peak.features.freq_high_hz,
                bandwidth_hz: peak.features.bandwidth_hz,
                cf_tail_ratio: peak.features.cf_tail_ratio,
                rep_rate: peak.features.rep_rate,
                is_cf: peak.features.is_cf,
                code: peak.code,
                notes: peak.notes,
            });
        }
    }

    let mut passes: Vec<PassInfo> = Vec::new();

    for (species, mut items) in by_species {
        items.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());

        // Running accumulators for the current pass group.
        // Notes are constant per species — carry the first sample's value.
        macro_rules! flush {
            ($cs:expr, $ce:expr, $sums:expr, $n:expr, $code:expr, $notes:expr) => {{
                let n = $n as f32;
                let mean_ph = $sums.0 / n;
                // Variance via E[x²] − E[x]²; .max(0) guards floating-point rounding.
                let peak_hz_std = (($sums.7 / n) - mean_ph * mean_ph).max(0.0).sqrt();
                passes.push(PassInfo {
                    species,
                    start_sec: $cs,
                    end_sec: $ce,
                    n_pulses: $n,
                    n_extra: 0,
                    mean_peak_hz:      mean_ph,
                    peak_hz_std,
                    mean_freq_low_hz:  $sums.1 / n,
                    mean_freq_high_hz: $sums.2 / n,
                    mean_bandwidth_hz: $sums.3 / n,
                    mean_cf_tail_ratio:$sums.4 / n,
                    mean_rep_rate:     $sums.5 / n,
                    is_cf:             $sums.6,
                    mean_energy_db: 0.0,
                    peak_energy_db: 0.0,
                    code: $code,
                    notes: $notes,
                    dubious: false,
                });
            }};
        }

        let s0 = &items[0];
        let mut cur_start = s0.start;
        let mut cur_end   = s0.end;
        let mut cur_code  = s0.code;
        let mut cur_notes = s0.notes;
        // sums: (peak_hz, freq_low, freq_high, bandwidth, cf_tail_ratio, rep_rate, any_cf, peak_hz_sq)
        let mut sums = (s0.peak_hz, s0.freq_low_hz, s0.freq_high_hz,
                        s0.bandwidth_hz, s0.cf_tail_ratio, s0.rep_rate, s0.is_cf,
                        s0.peak_hz * s0.peak_hz);
        let mut count = 1usize;

        for s in &items[1..] {
            if s.start - cur_end <= max_gap_sec {
                if s.end > cur_end { cur_end = s.end; }
                sums.0 += s.peak_hz;
                sums.1 += s.freq_low_hz;
                sums.2 += s.freq_high_hz;
                sums.3 += s.bandwidth_hz;
                sums.4 += s.cf_tail_ratio;
                sums.5 += s.rep_rate;
                sums.6 |= s.is_cf;
                sums.7 += s.peak_hz * s.peak_hz;
                count += 1;
            } else {
                flush!(cur_start, cur_end, sums, count, cur_code, cur_notes);
                cur_start = s.start;
                cur_end   = s.end;
                cur_code  = s.code;
                cur_notes = s.notes;
                sums = (s.peak_hz, s.freq_low_hz, s.freq_high_hz,
                        s.bandwidth_hz, s.cf_tail_ratio, s.rep_rate, s.is_cf,
                        s.peak_hz * s.peak_hz);
                count = 1;
            }
        }
        flush!(cur_start, cur_end, sums, count, cur_code, cur_notes);
    }

    passes.sort_by(|a, b| a.start_sec.partial_cmp(&b.start_sec).unwrap());
    passes
}

// ── CSV output ────────────────────────────────────────────────────────────────

/// Try to extract ISO date and time strings from a file path like
/// `data/20260322_190000.WAV` → (`"2026-03-22"`, `"19:00:00"`).
/// Matches the pattern `YYYYMMDD_HHMMSS` anywhere in the filename stem.
/// Returns empty strings when the pattern is not found.
fn parse_stem_datetime(path: &str) -> (String, String) {
    // Get the filename component, strip any extension.
    let name = path.rsplit('/').next().unwrap_or(path)
                   .rsplit('\\').next().unwrap_or(path);
    let base = name.rsplit('.').nth(1).map(|_| {
        // has a dot — drop everything from the last dot
        &name[..name.rfind('.').unwrap()]
    }).unwrap_or(name);

    // Search for YYYYMMDD_HHMMSS anywhere in the base name.
    for i in 0..base.len().saturating_sub(14) {
        let s = &base[i..i + 15];
        let b = s.as_bytes();
        if b[..8].iter().all(|c| c.is_ascii_digit())
            && b[8] == b'_'
            && b[9..].iter().all(|c| c.is_ascii_digit())
        {
            let date = format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8]);
            let time = format!("{}:{}:{}", &s[9..11], &s[11..13], &s[13..15]);
            return (date, time);
        }
    }
    (String::new(), String::new())
}

/// Write a CSV with one row per species pass.
///
/// Columns: filename, date, time, pass, start_s, end_s, duration_ms,
///          n_pulses, n_extra, mean_peak_khz, freq_low_khz, freq_high_khz,
///          bandwidth_khz, cf_tail_ratio, rep_rate_hz, is_cf,
///          mean_energy_db, peak_energy_db, species, dubious
pub fn write_csv(
    stem: &str,
    path: &str,
    passes: &[PassInfo],
) -> std::io::Result<()> {
    let out_path = format!("{}_detections.csv", stem);
    let file = std::fs::File::create(&out_path)?;
    let mut w = std::io::BufWriter::new(file);

    let (date, time) = parse_stem_datetime(path);
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);

    writeln!(
        w,
        "filename,date,time,pass,start_s,end_s,duration_ms,\
         n_pulses,n_extra,mean_peak_khz,peak_hz_std_khz,freq_low_khz,freq_high_khz,\
         bandwidth_khz,cf_tail_ratio,rep_rate_hz,is_cf,\
         mean_energy_db,peak_energy_db,code,species,notes,dubious,confidence"
    )?;

    for (i, p) in passes.iter().enumerate() {
        // Species and notes can contain commas — wrap in double quotes.
        let species_quoted = format!("\"{}\"", p.species.replace('"', "\"\""));
        let notes_quoted   = format!("\"{}\"", p.notes.replace('"', "\"\""));
        writeln!(
            w,
            "{},{},{},{},{:.3},{:.3},{:.0},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.2},{},{:.2},{:.2},{},{},{},{},{:.2}",
            filename,
            date, time,
            i + 1,
            p.start_sec, p.end_sec,
            (p.end_sec - p.start_sec) * 1000.0,
            p.n_pulses, p.n_extra,
            p.mean_peak_hz  / 1000.0,
            p.peak_hz_std   / 1000.0,
            p.mean_freq_low_hz  / 1000.0,
            p.mean_freq_high_hz / 1000.0,
            p.mean_bandwidth_hz / 1000.0,
            p.mean_cf_tail_ratio,
            p.mean_rep_rate,
            p.is_cf,
            p.mean_energy_db, p.peak_energy_db,
            p.code,
            species_quoted,
            notes_quoted,
            p.dubious,
            p.confidence(),
        )?;
    }

    println!("Detection CSV saved to:     {}", out_path);
    Ok(())
}

// ── PNG output ────────────────────────────────────────────────────────────────

/// Write a spectrogram PNG.
///
/// `spec_bytes` is window-major (`bytes[w * freq_bins + b]`), with values
/// already normalised to 0–255.
pub fn write_png(
    stem: &str,
    spec_bytes: &[u8],
    detected: &[bool],
    n_windows: usize,
    freq_bins: usize,
    bin_low: usize,
    bin_high: usize,
) -> Result<(), image::ImageError> {
    const MARKER_H: u32 = 8;
    let w = n_windows as u32;
    let h = freq_bins as u32 + MARKER_H;
    let mut img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);

    for (x, win_bytes) in spec_bytes.chunks(freq_bins).enumerate().take(n_windows) {
        // Detection marker strip
        let mc = if detected[x] { Rgb([220u8, 50, 50]) } else { Rgb([30u8, 30, 30]) };
        for y in 0..MARKER_H {
            img.put_pixel(x as u32, y, mc);
        }
        // Spectrogram pixels
        for (bin, &byte) in win_bytes.iter().enumerate() {
            let [r, g, b] = colormap(byte as f32 / 255.0);
            let y = MARKER_H + (freq_bins as u32 - 1 - bin as u32);
            img.put_pixel(x as u32, y, Rgb([r, g, b]));
        }
    }

    // Vertical white lines at call group boundaries
    for x in 1..w {
        if detected[x as usize] != detected[x as usize - 1] {
            for y in 0..h {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
    }

    // Horizontal lines at bat-band boundaries
    let y_low = MARKER_H + (freq_bins as u32 - 1 - bin_low as u32);
    let y_high = MARKER_H + (freq_bins as u32 - 1 - bin_high as u32);
    for x in 0..w {
        img.put_pixel(x, y_low, Rgb([255, 255, 255]));
        img.put_pixel(x, y_high, Rgb([255, 255, 255]));
    }

    let path = format!("{}_spectrogram.png", stem);
    img.save(&path)?;
    println!("Spectrogram PNG saved to:   {}", path);
    Ok(())
}

// ── HTML output ───────────────────────────────────────────────────────────────

const CSS: &str = include_str!("static/spectrogram.css");

const JS: &str = include_str!("static/spectrogram.js");


// ── Helpers ───────────────────────────────────────────────────────────────────

/// Encode `data` as base64, streaming directly to `w` (no intermediate String).
fn write_base64<W: std::io::Write>(w: &mut W, data: &[u8]) -> std::io::Result<()> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = [0u8; 4];
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        buf[0] = T[(n >> 18) as usize];
        buf[1] = T[((n >> 12) & 63) as usize];
        buf[2] = if chunk.len() > 1 { T[((n >> 6) & 63) as usize] } else { b'=' };
        buf[3] = if chunk.len() > 2 { T[(n & 63) as usize] } else { b'=' };
        w.write_all(&buf)?;
    }
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Wrap `s` in a JS double-quoted string literal, escaping backslashes and quotes.
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── HTML writer ───────────────────────────────────────────────────────────────

/// Write a self-contained interactive HTML spectrogram viewer.
///
/// `spec_bytes` is window-major (`bytes[w * freq_bins + b]`), values 0–255.
pub fn write_html(
    stem: &str,
    sample_rate: f32,
    window_size: usize,
    n_windows: usize,
    freq_bins: usize,
    hz_per_bin: f32,
    spec_bytes: &[u8],
    detected: &[bool],
    calls: &[CallGroupInfo],
    passes: &[PassInfo],
) -> std::io::Result<()> {
    let out_path = format!("{}_spectrogram.html", stem);
    let file = std::fs::File::create(&out_path)?;
    let mut w = std::io::BufWriter::new(file);

    let duration_sec = n_windows as f32 * window_size as f32 / sample_rate;
    let n_pulses: usize = calls.iter().map(|c| c.peaks.len()).sum();

    // ── Head ─────────────────────────────────────────────────────────────────
    w.write_all(b"<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n")?;
    write!(w, "<title>Bat Spectrogram \u{2014} {}</title>\n", stem)?;
    write!(w, "<style>\n{}</style>\n", CSS)?;
    w.write_all(b"</head>\n<body>\n")?;

    // ── Header ───────────────────────────────────────────────────────────────
    write!(w, "<h1>Bat Spectrogram \u{2014} {}</h1>\n", stem)?;
    write!(
        w,
        "<p class=\"meta\">Sample rate: {} Hz &nbsp;|&nbsp; \
         Duration: {:.1} s &nbsp;|&nbsp; \
         {} windows ({}-point FFT) &nbsp;|&nbsp; \
         {} pulse(s) &rarr; {} pass(es)</p>\n",
        sample_rate as u32, duration_sec, n_windows, window_size,
        n_pulses, passes.len(),
    )?;
    w.write_all(
        b"<p class=\"help\">Scroll: zoom time &nbsp;|&nbsp; \
          Shift+scroll: zoom frequency &nbsp;|&nbsp; \
          Drag: pan &nbsp;|&nbsp; \
          Shift+drag: select time range (filters table) &nbsp;|&nbsp; \
          Esc: clear selection &nbsp;|&nbsp; \
          Double-click: reset view &nbsp;|&nbsp; \
          Click pass row to zoom</p>\n",
    )?;

    // ── Canvas ────────────────────────────────────────────────────────────────
    w.write_all(b"<div id=\"wrap\"><canvas id=\"cv\"></canvas></div>\n")?;
    w.write_all(b"<div id=\"info\">&nbsp;</div>\n")?;

    // ── Passes table ──────────────────────────────────────────────────────────
    w.write_all(b"<div id=\"calls\">\n")?;
    w.write_all(b"<h2>Species passes <span style=\"color:#555;font-size:11px\">(click to zoom)</span></h2>\n")?;
    w.write_all(
        b"<table><thead><tr>\
          <th>#</th><th>Time</th><th>Duration</th>\
          <th>Pulses</th><th>Mean peak</th><th>Code</th><th>Species</th><th>Conf.</th>\
          </tr></thead><tbody>\n",
    )?;

    for (i, pass) in passes.iter().enumerate() {
        let duration_ms = (pass.end_sec - pass.start_sec) * 1000.0;
        let conf = pass.confidence();
        let row_class = if pass.dubious { "dubious" } else { "" };
        let pulses_cell = if pass.n_extra > 0 {
            format!(
                "{} <span style=\"color:#668;font-size:10px\">(+{}&nbsp;nearby)</span>",
                pass.n_pulses, pass.n_extra
            )
        } else {
            format!("{}", pass.n_pulses)
        };
        let species_cell = if pass.dubious {
            format!(
                "{} <span style=\"color:#555;font-size:10px\">(nested&nbsp;&#x2753;)</span>",
                pass.species
            )
        } else {
            pass.species.to_string()
        };
        // Colour the confidence badge: green ≥ 0.75, amber ≥ 0.40, red below.
        let conf_color = if conf >= 0.75 { "#3a3" } else if conf >= 0.40 { "#963" } else { "#933" };
        let conf_cell = format!(
            "<span style=\"display:inline-block;padding:1px 5px;border-radius:2px;\
             font-size:10px;background:{};color:#eee\">{:.0}%</span>",
            conf_color, conf * 100.0
        );
        write!(
            w,
            "<tr data-t0=\"{:.3}\" data-t1=\"{:.3}\" class=\"{}\">\
             <td>{}</td>\
             <td>{:.1}&ndash;{:.1}s</td>\
             <td>{:.0}ms</td>\
             <td>{}</td>\
             <td>{:.1}kHz</td>\
             <td><code style=\"color:#adf\">{}</code></td>\
             <td>{}</td>\
             <td>{}</td>\
             </tr>\n",
            pass.start_sec, pass.end_sec,
            row_class,
            i + 1,
            pass.start_sec, pass.end_sec,
            duration_ms,
            pulses_cell,
            pass.mean_peak_hz / 1000.0,
            pass.code,
            species_cell,
            conf_cell,
        )?;
    }
    w.write_all(b"</tbody></table>\n</div>\n")?;

    // ── Script: dynamic data ──────────────────────────────────────────────────
    w.write_all(b"<script>\n")?;

    write!(
        w,
        "const D={{nW:{},nB:{},sr:{},ws:{},hpb:{:.6}}};\n",
        n_windows, freq_bins, sample_rate as u32, window_size, hz_per_bin
    )?;

    // Detected array (compact 0/1)
    w.write_all(b"D.det=[")?;
    for (i, &d) in detected.iter().enumerate() {
        if i > 0 { w.write_all(b",")?; }
        w.write_all(if d { b"1" } else { b"0" })?;
    }
    w.write_all(b"];\n")?;

    // Calls array — start/end windows, used only for boundary lines in the spectrogram
    w.write_all(b"D.calls=[")?;
    for (i, call) in calls.iter().enumerate() {
        if i > 0 { w.write_all(b",")?; }
        write!(w, r#"{{"s":{},"e":{}}}"#, call.start_win, call.end_win)?;
    }
    w.write_all(b"];\n")?;

    // Passes array — time range + code + species name, used for mouse-over labels
    w.write_all(b"D.passes=[")?;
    for (i, pass) in passes.iter().enumerate() {
        if i > 0 { w.write_all(b",")?; }
        write!(w, r#"{{"t0":{:.3},"t1":{:.3},"co":{},"sp":{}}}"#,
            pass.start_sec, pass.end_sec,
            js_str(pass.code), js_str(pass.species))?;
    }
    w.write_all(b"];\n")?;

    // Base64 spectrogram data → D.bytes
    w.write_all(b"(function(){const s='")?;
    write_base64(&mut w, spec_bytes)?;
    w.write_all(b"';const b=atob(s);const a=new Uint8Array(b.length);for(let i=0;i<b.length;i++)a[i]=b.charCodeAt(i);D.bytes=a;})();\n")?;

    // Static rendering + interaction code
    w.write_all(JS.as_bytes())?;
    w.write_all(b"</script>\n</body>\n</html>\n")?;

    println!("Interactive HTML saved to:  {}", out_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── colormap ──────────────────────────────────────────────────────────────

    #[test]
    fn colormap_black_at_zero() {
        assert_eq!(colormap(0.0), [0, 0, 0]);
    }

    #[test]
    fn colormap_red_at_one() {
        assert_eq!(colormap(1.0), [255, 0, 0]);
    }

    #[test]
    fn colormap_blue_at_quarter() {
        // At t=0.25 we are at the top of the blue ramp → full blue
        assert_eq!(colormap(0.25), [0, 0, 255]);
    }

    #[test]
    fn colormap_cyan_at_half() {
        // At t=0.5 we are at the top of the green ramp → cyan (0, 255, 255)
        assert_eq!(colormap(0.5), [0, 255, 255]);
    }

    #[test]
    fn colormap_yellow_at_three_quarters() {
        // At t=0.75 we are at the top of the red ramp → yellow (255, 255, 0)
        assert_eq!(colormap(0.75), [255, 255, 0]);
    }

    #[test]
    fn colormap_clamps_below_zero() {
        assert_eq!(colormap(-1.0), colormap(0.0));
    }

    #[test]
    fn colormap_clamps_above_one() {
        assert_eq!(colormap(2.0), colormap(1.0));
    }

    // ── write_base64 ──────────────────────────────────────────────────────────

    fn b64(data: &[u8]) -> String {
        let mut out = Vec::new();
        write_base64(&mut out, data).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn base64_empty() {
        assert_eq!(b64(b""), "");
    }

    #[test]
    fn base64_one_byte() {
        // 0x4d = 'M' → base64 "TQ=="
        assert_eq!(b64(b"M"), "TQ==");
    }

    #[test]
    fn base64_two_bytes() {
        // "Ma" → "TWE="
        assert_eq!(b64(b"Ma"), "TWE=");
    }

    #[test]
    fn base64_three_bytes_no_padding() {
        // "Man" → "TWFu"
        assert_eq!(b64(b"Man"), "TWFu");
    }

    #[test]
    fn base64_known_string() {
        // Standard test vector: "Hello" → "SGVsbG8="
        assert_eq!(b64(b"Hello"), "SGVsbG8=");
    }

    // ── html_escape ───────────────────────────────────────────────────────────

    #[test]
    fn html_escape_plain() {
        assert_eq!(html_escape("hello"), "hello");
    }

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("<b>\"foo\" & 'bar'</b>"), "&lt;b&gt;&quot;foo&quot; &amp; 'bar'&lt;/b&gt;");
    }
}
