import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AgGridReact } from "ag-grid-react";
import type { ColDef, GridReadyEvent, CellMouseOverEvent, CellClickedEvent, CellValueChangedEvent } from "ag-grid-community";
import { ModuleRegistry, AllCommunityModule, themeQuartz } from "ag-grid-community";
import type { PassRecord, AnnotationRow } from "../types";

ModuleRegistry.registerModules([AllCommunityModule]);

const darkTheme = themeQuartz.withParams({
  backgroundColor: "#1e1e1e",
  foregroundColor: "#cccccc",
  borderColor: "#333333",
  headerBackgroundColor: "#242424",
  headerTextColor: "#888888",
  rowHoverColor: "#252525",
  accentColor: "#44aaff",
  cellTextColor: "#cccccc",
  fontSize: 11,
  rowHeight: 26,
  headerHeight: 28,
});

interface Props {
  recordingId: string | null;
  passes: PassRecord[];
  /** Annotations previously saved for this recording — used to pre-fill review columns. */
  savedAnnotations: AnnotationRow[];
  onSave: (edits: AnnotationRow[]) => void;
  saving: boolean;
  /** Called when the user clicks a row — allows the parent to highlight that region in the spectrogram. */
  onRowClick?: (t0: number, t1: number) => void;
  /** Called on row hover (null to clear). */
  onRowHover?: (range: { t0: number; t1: number } | null) => void;
}

interface GridRow extends PassRecord {
  review_status: string;
  reviewed_code: string;
  reviewed_species: string;
  keep: boolean;
  review_notes: string;
}

function confColor(conf: number): string {
  if (conf >= 0.75) return "#3a3";
  if (conf >= 0.40) return "#963";
  return "#933";
}

function ConfBadge({ value }: { value: number }) {
  return (
    <span style={{
      display: "inline-block", padding: "1px 6px", borderRadius: 3,
      fontSize: 11, background: confColor(value), color: "#eee",
    }}>
      {(value * 100).toFixed(0)}%
    </span>
  );
}

function DubiousCell({ value }: { value: boolean }) {
  return value ? <span style={{ color: "#f84", fontSize: 11 }}>dubious</span> : null;
}

export default function PassGrid({ recordingId, passes, savedAnnotations, onSave, saving, onRowClick, onRowHover }: Props) {
  const gridRef = useRef<AgGridReact<GridRow>>(null);
  const [rowData, setRowData] = useState<GridRow[]>([]);

  // Rebuild grid rows whenever the recording or its saved annotations change.
  useEffect(() => {
    const saved = new Map(savedAnnotations.map(a => [a.pass_idx, a]));
    setRowData(
      passes.map((p) => {
        const ann = saved.get(p.idx);
        return {
          ...p,
          review_status:    ann?.review_status    ?? "",
          reviewed_code:    ann?.reviewed_code    ?? p.code,
          reviewed_species: ann?.reviewed_species ?? p.species,
          keep:             ann?.keep             ?? !p.dubious,
          review_notes:     ann?.notes            ?? "",
        };
      })
    );
  }, [passes, savedAnnotations]);

  const handleSave = useCallback(() => {
    if (!recordingId || !gridRef.current) return;
    const edits: AnnotationRow[] = [];
    gridRef.current.api.forEachNode((node) => {
      const r = node.data!;
      edits.push({
        recording_id:     recordingId,
        pass_idx:         r.idx,
        // Analysis fields — included so the output CSV is self-contained
        start_sec:        r.start_sec,
        end_sec:          r.end_sec,
        n_pulses:         r.n_pulses,
        n_extra:          r.n_extra,
        mean_peak_khz:    r.mean_peak_khz,
        freq_low_khz:     r.freq_low_khz,
        freq_high_khz:    r.freq_high_khz,
        bandwidth_khz:    r.bandwidth_khz,
        rep_rate:         r.rep_rate,
        call_dur_ms:      r.call_dur_ms,
        mean_energy_db:   r.mean_energy_db,
        confidence:       r.confidence,
        auto_code:        r.code,
        auto_species:     r.species,
        is_cf:            r.is_cf,
        dubious:          r.dubious,
        // Review fields
        review_status:    r.review_status,
        reviewed_code:    r.reviewed_code,
        reviewed_species: r.reviewed_species,
        keep:             r.keep,
        notes:            r.review_notes,
        updated_at:       "",
      });
    });
    onSave(edits);
  }, [recordingId, onSave]);

  // Hover → spectrogram green band.  Debounce the clear by 60 ms so moving
  // between cells of the same row doesn't flicker.
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastHoverIdxRef = useRef<number | null>(null);

  const handleCellMouseOver = useCallback((e: CellMouseOverEvent<GridRow>) => {
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    if (e.rowIndex === lastHoverIdxRef.current) return;
    lastHoverIdxRef.current = e.rowIndex ?? null;
    if (e.data) onRowHover?.({ t0: e.data.start_sec, t1: e.data.end_sec });
  }, [onRowHover]);

  const handleCellMouseOut = useCallback(() => {
    hoverTimerRef.current = setTimeout(() => {
      // Don't update hover state while a cell editor is open — the re-render
      // that follows setHoverRange() causes AG Grid to close the editor.
      if (gridRef.current?.api.getEditingCells().length) return;
      lastHoverIdxRef.current = null;
      onRowHover?.(null);
    }, 60);
  }, [onRowHover]);

  const onGridReady = useCallback((_e: GridReadyEvent) => {
    gridRef.current?.api.sizeColumnsToFit();
  }, []);

  const handleCellClicked = useCallback((e: CellClickedEvent<GridRow>) => {
    // Don't interfere with editable cells — let AG Grid handle the editor.
    if (e.colDef.editable) return;
    if (e.data && onRowClick) {
      onRowClick(e.data.start_sec, e.data.end_sec);
    }
  }, [onRowClick]);

  // Keep rowData React state in sync with AG Grid's internal mutations.
  // Without this, AG Grid reconciles against stale rowData and reverts edits.
  const handleCellValueChanged = useCallback((e: CellValueChangedEvent<GridRow>) => {
    if (!e.data) return;
    setRowData(rows => rows.map(r => r.idx === e.data!.idx ? { ...e.data! } : r));
  }, []);

  const colDefs = useMemo<ColDef<GridRow>[]>(() => [
    { field: "idx", headerName: "#", width: 46, pinned: "left" },
    {
      headerName: "Time",
      valueGetter: (p) =>
        p.data ? `${p.data.start_sec.toFixed(1)}–${p.data.end_sec.toFixed(1)}s` : "",
      width: 90,
    },
    {
      headerName: "Dur",
      valueGetter: (p) => p.data ? p.data.end_sec - p.data.start_sec : null,
      valueFormatter: (p) => p.value != null ? `${(p.value as number).toFixed(2)}s` : "",
      width: 65,
    },
    { field: "n_pulses", headerName: "Pulses", width: 70 },
    {
      field: "mean_peak_khz", headerName: "Peak kHz", width: 80,
      valueFormatter: (p) => p.value?.toFixed(1) ?? "",
    },
    { field: "code", headerName: "Code", width: 80 },
    { field: "species", headerName: "Species", flex: 1, minWidth: 160 },
    { field: "confidence", headerName: "Conf", width: 70, cellRenderer: ConfBadge },
    {
      field: "dubious", headerName: "Dubious", width: 75, cellRenderer: DubiousCell,
      headerTooltip: "Auto-detected quality flag — read only",
    },
    // ── Editable review columns (single-click to edit) ─────────────────────
    { field: "reviewed_code",    headerName: "★ Rev. code",    width: 90,  editable: true, cellStyle: { color: "#adf" } },
    { field: "reviewed_species", headerName: "★ Rev. species", flex: 1, minWidth: 140, editable: true, cellStyle: { color: "#adf" } },
    { field: "keep",         headerName: "★ Keep",  width: 65, editable: true, cellRenderer: "agCheckboxCellRenderer" },
    { field: "review_notes", headerName: "★ Notes", flex: 1, minWidth: 120, editable: true, cellStyle: { color: "#adf" } },
  ], []);

  if (!recordingId) {
    return <div style={styles.empty}>No recording loaded.</div>;
  }

  return (
    <div style={styles.root}>
      <div style={styles.toolbar}>
        <span style={styles.title}>Passes — {recordingId}</span>
        <button style={styles.saveBtn} onClick={handleSave} disabled={saving}>
          {saving ? "Saving…" : "Save review"}
        </button>
      </div>
      <div style={{ flex: 1, overflow: "hidden", height: 0 }}>
        <div style={{ height: "100%", width: "100%" }}>
          <AgGridReact<GridRow>
            ref={gridRef}
            rowData={rowData}
            columnDefs={colDefs}
            onGridReady={onGridReady}
            onCellClicked={handleCellClicked}
            onCellValueChanged={handleCellValueChanged}
            onCellMouseOver={handleCellMouseOver}
            onCellMouseOut={handleCellMouseOut}
            theme={darkTheme}
            singleClickEdit
            stopEditingWhenCellsLoseFocus={false}
          />
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex", flexDirection: "column", height: "100%",
    background: "#1e1e1e", borderLeft: "1px solid #333", overflow: "hidden",
  },
  toolbar: {
    display: "flex", alignItems: "center", padding: "6px 10px",
    borderBottom: "1px solid #333", gap: 8, flexShrink: 0,
  },
  title: {
    flex: 1, fontSize: 11, color: "#888",
    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
  },
  saveBtn: {
    background: "#1a4a1a", color: "#8d8", border: "1px solid #3a6a3a",
    borderRadius: 3, padding: "3px 10px", fontSize: 11, cursor: "pointer", flexShrink: 0,
  },
  empty: {
    display: "flex", alignItems: "center", justifyContent: "center",
    height: "100%", fontSize: 12, color: "#444",
    background: "#1e1e1e", borderLeft: "1px solid #333",
  },
};
