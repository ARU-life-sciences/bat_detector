// ── Types mirroring the Rust structs returned by Tauri commands ───────────────

export interface RecordingListItem {
  file_name: string;
  path: string;
  size_bytes: number;
}

/** Serialisable pass record — mirrors bat_detector::api::PassRecord. */
export interface PassRecord {
  idx: number;
  code: string;
  species: string;
  notes: string;
  start_sec: number;
  end_sec: number;
  n_pulses: number;
  n_extra: number;
  mean_peak_khz: number;
  peak_hz_std_khz: number;
  freq_low_khz: number;
  freq_high_khz: number;
  bandwidth_khz: number;
  cf_tail_ratio: number;
  rep_rate: number;
  is_cf: boolean;
  call_dur_ms: number;
  mean_energy_db: number;
  peak_energy_db: number;
  dubious: boolean;
  confidence: number;
}

/** Full analysis result for one recording. */
export interface RecordingPayload {
  file_name: string;
  sample_rate: number;
  duration_sec: number;
  passes: PassRecord[];
  /** Absolute local path to the generated HTML file. Use convertFileSrc() to load it. */
  html_path: string;
}

/**
 * One annotation row — mirrors AnnotationRow in Rust and a line in annotations.csv.
 *
 * Analysis fields (start_sec … dubious) are populated on first save from the
 * PassGrid.  Rows loaded from older CSV files that predate this schema will have
 * zeroes/false for those fields.
 */
export interface AnnotationRow {
  recording_id: string;
  pass_idx: number;
  // ── Analysis (from PassRecord, saved alongside review data) ──────────────
  start_sec: number;
  end_sec: number;
  n_pulses: number;
  n_extra: number;
  mean_peak_khz: number;
  freq_low_khz: number;
  freq_high_khz: number;
  bandwidth_khz: number;
  rep_rate: number;
  call_dur_ms: number;
  mean_energy_db: number;
  confidence: number;
  auto_code: string;
  auto_species: string;
  is_cf: boolean;
  dubious: boolean;
  // ── Review (editable by the user) ────────────────────────────────────────
  review_status: string;
  reviewed_code: string;
  reviewed_species: string;
  keep: boolean;
  notes: string;
  updated_at: string;
}
