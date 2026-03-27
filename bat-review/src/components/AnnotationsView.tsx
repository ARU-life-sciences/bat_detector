import React, { useCallback, useMemo, useRef, useState, useEffect } from "react";
import { AgGridReact } from "ag-grid-react";
import type { ColDef, GridReadyEvent } from "ag-grid-community";
import { themeQuartz } from "ag-grid-community";
import type { AnnotationRow } from "../types";

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
  annotations: AnnotationRow[];
  onSave: (rows: AnnotationRow[]) => void;
  saving: boolean;
}

export default function AnnotationsView({ annotations, onSave, saving }: Props) {
  const gridRef = useRef<AgGridReact<AnnotationRow>>(null);
  const [rowData, setRowData] = useState<AnnotationRow[]>([]);

  useEffect(() => {
    setRowData([...annotations]);
  }, [annotations]);

  const handleSave = useCallback(() => {
    if (!gridRef.current) return;
    const rows: AnnotationRow[] = [];
    gridRef.current.api.forEachNode((node) => {
      if (node.data) rows.push({ ...node.data, updated_at: "" });
    });
    onSave(rows);
  }, [onSave]);

  const onGridReady = useCallback((_e: GridReadyEvent) => {
    gridRef.current?.api.sizeColumnsToFit();
  }, []);

  const colDefs = useMemo<ColDef<AnnotationRow>[]>(() => [
    {
      field: "recording_id", headerName: "Recording", pinned: "left",
      width: 200, cellStyle: { color: "#aaa" },
      valueFormatter: (p) => p.value?.split("/").pop() ?? p.value,
    },
    { field: "pass_idx", headerName: "#", width: 46 },
    // ── Analysis (read-only) ──────────────────────────────────────────────────
    {
      headerName: "Time",
      valueGetter: (p) => p.data ? `${p.data.start_sec.toFixed(1)}–${p.data.end_sec.toFixed(1)}s` : "",
      width: 90, cellStyle: { color: "#888" },
    },
    { field: "n_pulses", headerName: "Pulses", width: 65, cellStyle: { color: "#888" } },
    {
      field: "mean_peak_khz", headerName: "Peak kHz", width: 80, cellStyle: { color: "#888" },
      valueFormatter: (p) => p.value ? p.value.toFixed(1) : "",
    },
    {
      field: "confidence", headerName: "Conf", width: 65, cellStyle: { color: "#888" },
      valueFormatter: (p) => p.value ? `${(p.value * 100).toFixed(0)}%` : "",
    },
    { field: "auto_code",    headerName: "Auto code",    width: 85,  cellStyle: { color: "#888" } },
    { field: "auto_species", headerName: "Auto species", flex: 1, minWidth: 140, cellStyle: { color: "#888" } },
    {
      field: "dubious", headerName: "Dub", width: 55, cellStyle: { color: "#f84" },
      valueFormatter: (p) => p.value ? "●" : "",
    },
    // ── Review (editable) ─────────────────────────────────────────────────────
    {
      field: "review_status", headerName: "Status", width: 120, editable: true,
      cellEditor: "agSelectCellEditor",
      cellEditorParams: { values: ["", "reviewed", "uncertain", "false_positive"] },
    },
    { field: "reviewed_code",    headerName: "Rev. code",    width: 90,  editable: true },
    { field: "reviewed_species", headerName: "Rev. species", flex: 1, minWidth: 140, editable: true },
    { field: "keep", headerName: "Keep", width: 60, editable: true, cellRenderer: "agCheckboxCellRenderer" },
    { field: "notes", headerName: "Notes", flex: 1, minWidth: 130, editable: true },
    { field: "updated_at", headerName: "Saved at", width: 155, cellStyle: { color: "#555" } },
  ], []);

  if (annotations.length === 0) {
    return (
      <div style={styles.empty}>
        No annotations saved yet.<br />
        Review recordings and click "Save review" to build up the database.
      </div>
    );
  }

  return (
    <div style={styles.root}>
      <div style={styles.toolbar}>
        <span style={styles.title}>
          All annotations — {annotations.length} pass{annotations.length !== 1 ? "es" : ""} across{" "}
          {new Set(annotations.map(a => a.recording_id)).size} recording{annotations.length !== 1 ? "s" : ""}
        </span>
        <button style={styles.saveBtn} onClick={handleSave} disabled={saving}>
          {saving ? "Saving…" : "Save all"}
        </button>
      </div>
      <div style={{ flex: 1, overflow: "hidden", height: 0 }}>
        <div style={{ height: "100%", width: "100%" }}>
          <AgGridReact<AnnotationRow>
            ref={gridRef}
            rowData={rowData}
            columnDefs={colDefs}
            onGridReady={onGridReady}
            theme={darkTheme}
          />
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex", flexDirection: "column", height: "100%",
    background: "#1e1e1e", overflow: "hidden",
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
    height: "100%", fontSize: 12, color: "#555", textAlign: "center",
    lineHeight: 1.8, background: "#1e1e1e",
  },
};
