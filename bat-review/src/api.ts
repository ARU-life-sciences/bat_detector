import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { RecordingListItem, RecordingPayload, AnnotationRow } from "./types";

/** Open a native folder-picker dialog and return the selected path, or null. */
export async function pickFolder(): Promise<string | null> {
  const result = await open({ directory: true, multiple: false });
  if (!result) return null;
  return Array.isArray(result) ? result[0] : result;
}

/** List all WAV files in `dir`. */
export async function listRecordings(dir: string): Promise<RecordingListItem[]> {
  return invoke<RecordingListItem[]>("list_recordings", { dir });
}

/** Analyse one WAV file and return full spectrogram payload. */
export async function analyzeRecording(
  path: string,
  threshold = 3.0,
  ratio = 1.05
): Promise<RecordingPayload> {
  return invoke<RecordingPayload>("analyze_recording", { path, threshold, ratio });
}

/** Return all saved annotation rows for a project folder. */
export async function loadAnnotations(projectDir: string): Promise<AnnotationRow[]> {
  return invoke<AnnotationRow[]>("load_annotations", { projectDir });
}

/**
 * Merge edits into annotations.csv.
 * Rows for recording_ids present in `edits` are replaced; all other recordings
 * are preserved.  Pass the full set of rows for each recording you're saving.
 */
export async function saveAnnotations(
  projectDir: string,
  edits: AnnotationRow[]
): Promise<void> {
  return invoke("save_annotations", { projectDir, edits });
}
