use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use bat_detector::api::{analyze_wav, AnalysisParams, PassRecord};

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct RecordingListItem {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
}

/// Payload returned to the frontend — note `html_path` (not html content).
/// The frontend loads the spectrogram via `convertFileSrc(html_path)`.
#[derive(Serialize)]
pub struct RecordingPayload {
    pub file_name: String,
    pub sample_rate: u32,
    pub duration_sec: f32,
    pub passes: Vec<PassRecord>,
    /// Absolute local path to the spectrogram HTML file. Use convertFileSrc() in the frontend.
    pub html_path: String,
}

/// One annotation row — mirrors the CSV columns and the TypeScript AnnotationRow type.
///
/// Column layout (v2, 24 columns):
///   recording_id, pass_idx,
///   start_sec, end_sec, n_pulses, n_extra, mean_peak_khz,
///   freq_low_khz, freq_high_khz, bandwidth_khz, rep_rate, call_dur_ms,
///   mean_energy_db, confidence, auto_code, auto_species, is_cf, dubious,
///   review_status, reviewed_code, reviewed_species, keep, notes, updated_at
///
/// Old v1 rows (8 columns) are read back with analysis fields defaulting to 0.
#[derive(Serialize, Deserialize, Clone)]
pub struct AnnotationRow {
    pub recording_id: String,
    pub pass_idx: u32,
    // Analysis fields
    pub start_sec: f32,
    pub end_sec: f32,
    pub n_pulses: u32,
    pub n_extra: u32,
    pub mean_peak_khz: f32,
    pub freq_low_khz: f32,
    pub freq_high_khz: f32,
    pub bandwidth_khz: f32,
    pub rep_rate: f32,
    pub call_dur_ms: f32,
    pub mean_energy_db: f32,
    pub confidence: f32,
    pub auto_code: String,
    pub auto_species: String,
    pub is_cf: bool,
    pub dubious: bool,
    // Review fields
    pub review_status: String,
    pub reviewed_code: String,
    pub reviewed_species: String,
    pub keep: bool,
    pub notes: String,
    pub updated_at: String,
}

/// Bump this whenever spectrogram.js or the HTML template changes so that
/// old cached HTML files are automatically regenerated on next analysis.
const CACHE_VERSION: u32 = 5;

/// What we write to `derived/<stem>.cache.json`.
#[derive(Serialize, Deserialize)]
struct CachedResult {
    #[serde(default)]
    cache_version: u32,
    file_name: String,
    sample_rate: u32,
    duration_sec: f32,
    passes: Vec<PassRecord>,
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// List all WAV files directly inside `dir` (non-recursive).
#[tauri::command]
fn list_recordings(dir: String) -> Result<Vec<RecordingListItem>, String> {
    let entries = fs::read_dir(&dir)
        .map_err(|e| format!("Cannot read '{}': {}", dir, e))?;

    let mut items: Vec<RecordingListItem> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".wav"))
        .map(|e| {
            let size_bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            RecordingListItem {
                file_name: e.file_name().to_string_lossy().into_owned(),
                path: e.path().to_string_lossy().into_owned(),
                size_bytes,
            }
        })
        .collect();

    items.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(items)
}

/// Analyse one WAV file and return structured pass data + a path to the
/// spectrogram HTML file.
///
/// Results are cached in `<wav_dir>/derived/`:
///   - `<stem>_spectrogram.html` — the viewer HTML
///   - `<stem>.cache.json`       — passes + metadata
///
/// If both cache files are newer than the WAV, the analysis is skipped.
#[tauri::command]
async fn analyze_recording(
    path: String,
    threshold: f32,
    ratio: f32,
) -> Result<RecordingPayload, String> {
    tokio::task::spawn_blocking(move || analyze_recording_sync(path, threshold, ratio))
        .await
        .map_err(|e| format!("Task panicked: {}", e))?
}

fn analyze_recording_sync(
    path: String,
    threshold: f32,
    ratio: f32,
) -> Result<RecordingPayload, String> {
    let wav_path = Path::new(&path);
    let stem = wav_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording");

    let derived_dir = wav_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("derived");

    let html_cache  = derived_dir.join(format!("{}_spectrogram.html", stem));
    let meta_cache  = derived_dir.join(format!("{}.cache.json", stem));

    let t0 = std::time::Instant::now();

    // ── Try loading from disk cache ───────────────────────────────────────────
    if let Some(payload) = try_load_cache(&path, &html_cache, &meta_cache) {
        eprintln!("[bat-review] CACHE HIT  {} — {:?}", stem, t0.elapsed());
        return Ok(payload);
    }

    eprintln!("[bat-review] cache miss, analysing {}", stem);

    // ── Run full analysis pipeline ────────────────────────────────────────────
    let t_analysis = std::time::Instant::now();
    let params = AnalysisParams { threshold, ratio };
    let result = analyze_wav(&path, &params)?;
    eprintln!("[bat-review]   analysis  {:>7.0} ms", t_analysis.elapsed().as_millis());

    let t_html = std::time::Instant::now();
    let html   = result.to_html();
    eprintln!("[bat-review]   to_html   {:>7.0} ms  ({:.1} KB)", t_html.elapsed().as_millis(), html.len() as f32 / 1024.0);

    // ── Write cache (best-effort — don't fail if directory is read-only) ──────
    let html_path_str = if fs::create_dir_all(&derived_dir).is_ok()
        && fs::write(&html_cache, html.as_bytes()).is_ok()
    {
        let cached = CachedResult {
            cache_version: CACHE_VERSION,
            file_name:    result.file_name.clone(),
            sample_rate:  result.sample_rate,
            duration_sec: result.duration_sec,
            passes:       result.passes.clone(),
        };
        if let Ok(json) = serde_json::to_string(&cached) {
            let _ = fs::write(&meta_cache, json);
        }
        html_cache.to_string_lossy().into_owned()
    } else {
        // Fall back: write to OS temp dir.
        let tmp = std::env::temp_dir().join(format!("{}_spectrogram.html", stem));
        let _ = fs::write(&tmp, html.as_bytes());
        tmp.to_string_lossy().into_owned()
    };

    eprintln!("[bat-review]   TOTAL     {:>7.0} ms", t0.elapsed().as_millis());

    Ok(RecordingPayload {
        file_name:    result.file_name,
        sample_rate:  result.sample_rate,
        duration_sec: result.duration_sec,
        passes:       result.passes,
        html_path:    html_path_str,
    })
}

fn try_load_cache(
    wav_path_str: &str,
    html_cache: &Path,
    meta_cache: &Path,
) -> Option<RecordingPayload> {
    let wav_mtime  = mtime(wav_path_str)?;
    let html_mtime = mtime(html_cache.to_str()?)?;
    let meta_mtime = mtime(meta_cache.to_str()?)?;

    if html_mtime <= wav_mtime || meta_mtime <= wav_mtime {
        return None;
    }

    let json   = fs::read_to_string(meta_cache).ok()?;
    let cached: CachedResult = serde_json::from_str(&json).ok()?;

    // Reject caches built by an older version of spectrogram.js / HTML template.
    if cached.cache_version != CACHE_VERSION {
        return None;
    }

    Some(RecordingPayload {
        file_name:    cached.file_name,
        sample_rate:  cached.sample_rate,
        duration_sec: cached.duration_sec,
        passes:       cached.passes,
        html_path:    html_cache.to_string_lossy().into_owned(),
    })
}

fn mtime(path: &str) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

// ── CSV helpers ───────────────────────────────────────────────────────────────

fn annotations_path(project_dir: &str) -> std::path::PathBuf {
    Path::new(project_dir).join("review").join("annotations.csv")
}

/// Minimal RFC-4180 CSV field splitter — handles `""` as an escaped `"`.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => { in_quotes = true; }
            ',' if !in_quotes => { fields.push(std::mem::take(&mut current)); }
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

fn read_annotations(project_dir: &str) -> Vec<AnnotationRow> {
    let content = match fs::read_to_string(annotations_path(project_dir)) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut rows = Vec::new();
    for line in content.lines().skip(1) {
        if line.trim().is_empty() { continue; }
        let f = split_csv_line(line);
        macro_rules! f32 { ($i:expr) => { f.get($i).and_then(|s| s.parse().ok()).unwrap_or(0.0) } }
        macro_rules! u32 { ($i:expr) => { f.get($i).and_then(|s| s.parse().ok()).unwrap_or(0u32) } }
        macro_rules! bool_{ ($i:expr) => { f.get($i).map(|s| s == "true").unwrap_or(false) } }
        macro_rules! str { ($i:expr) => { f.get($i).cloned().unwrap_or_default() } }

        if f.len() >= 24 {
            // v2 layout (with analysis fields)
            rows.push(AnnotationRow {
                recording_id:     str!(0),
                pass_idx:         u32!(1),
                start_sec:        f32!(2),
                end_sec:          f32!(3),
                n_pulses:         u32!(4),
                n_extra:          u32!(5),
                mean_peak_khz:    f32!(6),
                freq_low_khz:     f32!(7),
                freq_high_khz:    f32!(8),
                bandwidth_khz:    f32!(9),
                rep_rate:         f32!(10),
                call_dur_ms:      f32!(11),
                mean_energy_db:   f32!(12),
                confidence:       f32!(13),
                auto_code:        str!(14),
                auto_species:     str!(15),
                is_cf:            bool_!(16),
                dubious:          bool_!(17),
                review_status:    str!(18),
                reviewed_code:    str!(19),
                reviewed_species: str!(20),
                keep:             bool_!(21),
                notes:            str!(22),
                updated_at:       str!(23),
            });
        } else if f.len() >= 8 {
            // v1 layout (review fields only) — analysis fields default to 0
            rows.push(AnnotationRow {
                recording_id:     str!(0),
                pass_idx:         u32!(1),
                start_sec: 0.0, end_sec: 0.0, n_pulses: 0, n_extra: 0,
                mean_peak_khz: 0.0, freq_low_khz: 0.0, freq_high_khz: 0.0,
                bandwidth_khz: 0.0, rep_rate: 0.0, call_dur_ms: 0.0,
                mean_energy_db: 0.0, confidence: 0.0,
                auto_code: String::new(), auto_species: String::new(),
                is_cf: false, dubious: false,
                review_status:    str!(2),
                reviewed_code:    str!(3),
                reviewed_species: str!(4),
                keep:             bool_!(5),
                notes:            str!(6),
                updated_at:       str!(7),
            });
        }
    }
    rows
}

const ANNOTATIONS_HEADER: &str =
    "recording_id,pass_idx,\
     start_sec,end_sec,n_pulses,n_extra,mean_peak_khz,\
     freq_low_khz,freq_high_khz,bandwidth_khz,rep_rate,call_dur_ms,\
     mean_energy_db,confidence,auto_code,auto_species,is_cf,dubious,\
     review_status,reviewed_code,reviewed_species,keep,notes,updated_at\n";

fn write_annotations(project_dir: &str, rows: &[AnnotationRow]) -> Result<(), String> {
    let path = annotations_path(project_dir);
    fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| format!("Cannot create review/ dir: {}", e))?;
    let mut csv = String::from(ANNOTATIONS_HEADER);
    for row in rows {
        let notes_esc = row.notes.replace('"', "\"\"");
        csv.push_str(&format!(
            "{},{},{:.3},{:.3},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.1},{:.2},{:.4},{},{},{},{},{},{},{},{},\"{}\",{}\n",
            row.recording_id, row.pass_idx,
            row.start_sec, row.end_sec,
            row.n_pulses, row.n_extra,
            row.mean_peak_khz, row.freq_low_khz, row.freq_high_khz,
            row.bandwidth_khz, row.rep_rate, row.call_dur_ms,
            row.mean_energy_db, row.confidence,
            row.auto_code, row.auto_species,
            if row.is_cf    { "true" } else { "false" },
            if row.dubious  { "true" } else { "false" },
            row.review_status, row.reviewed_code, row.reviewed_species,
            if row.keep { "true" } else { "false" },
            notes_esc, row.updated_at,
        ));
    }
    fs::write(&path, csv).map_err(|e| format!("Cannot write annotations: {}", e))
}

// ── Annotation commands ───────────────────────────────────────────────────────

/// Return all saved annotation rows for a project folder.
#[tauri::command]
fn load_annotations(project_dir: String) -> Vec<AnnotationRow> {
    read_annotations(&project_dir)
}

/// Merge `edits` into `<project_dir>/review/annotations.csv`.
///
/// All rows whose `recording_id` appears in `edits` are replaced; rows for
/// other recordings are preserved.  `updated_at` is always set by the backend.
#[tauri::command]
async fn save_annotations(
    project_dir: String,
    edits: Vec<AnnotationRow>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let ids: HashSet<String> = edits.iter().map(|r| r.recording_id.clone()).collect();

        let mut rows: Vec<AnnotationRow> = read_annotations(&project_dir)
            .into_iter()
            .filter(|r| !ids.contains(&r.recording_id))
            .collect();

        let now = chrono_now();
        for mut edit in edits {
            edit.updated_at = now.clone();
            rows.push(edit);
        }

        write_annotations(&project_dir, &rows)
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn chrono_now() -> String {
    use std::time::UNIX_EPOCH;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let tod  = secs % 86400;
    let (y, mo, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d,
            tod/3600, (tod%3600)/60, tod%60)
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut y = 1970u64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let dy = if leap { 366 } else { 365 };
        if days < dy { break; }
        days -= dy;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [31u64, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u64;
    for &dm in &months {
        if days < dm { break; }
        days -= dm;
        mo += 1;
    }
    (y, mo, days + 1)
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            list_recordings,
            analyze_recording,
            load_annotations,
            save_annotations,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Bat Review");
}
