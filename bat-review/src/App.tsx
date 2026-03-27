import { useState, useCallback, useMemo, useRef } from "react";
import RecordingList from "./components/RecordingList";
import SpectrogramViewer from "./components/SpectrogramViewer";
import PassGrid from "./components/PassGrid";
import AnnotationsView from "./components/AnnotationsView";
import AudioPlayer, { type AudioPlayerHandle } from "./components/AudioPlayer";
import {
  pickFolder,
  listRecordings,
  analyzeRecording,
  loadAnnotations,
  saveAnnotations,
} from "./api";
import type { RecordingListItem, RecordingPayload, AnnotationRow } from "./types";

const DEFAULT_THRESHOLD = 3.0;
const DEFAULT_RATIO     = 1.05;

// ── Resizable divider ─────────────────────────────────────────────────────────

interface DividerProps {
  onDrag: (delta: number) => void;
  axis?: "col" | "row";
}

function Divider({ onDrag, axis = "col" }: DividerProps) {
  const [active, setActive] = useState(false);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      setActive(true);
      let prev = axis === "col" ? e.clientX : e.clientY;

      const onMove = (ev: MouseEvent) => {
        const curr = axis === "col" ? ev.clientX : ev.clientY;
        onDrag(curr - prev);
        prev = curr;
      };
      const onUp = () => {
        setActive(false);
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [onDrag, axis]
  );

  const isRow = axis === "row";
  return (
    <div
      style={{
        [isRow ? "height" : "width"]: 5,
        flexShrink: 0,
        cursor: isRow ? "row-resize" : "col-resize",
        background: active ? "#4af" : "#2a2a2a",
        transition: "background 0.1s",
        zIndex: 10,
      }}
      onMouseDown={onMouseDown}
    />
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

type RightView = "recording" | "all-annotations";

export default function App() {
  const [projectDir, setProjectDir]     = useState<string | null>(null);
  const [recordings, setRecordings]     = useState<RecordingListItem[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [payload, setPayload]           = useState<RecordingPayload | null>(null);
  const [analysing, setAnalysing]       = useState(false);
  const [saving, setSaving]             = useState(false);
  const [error, setError]               = useState<string | null>(null);

  // All saved annotations for the current project folder.
  const [allAnnotations, setAllAnnotations] = useState<AnnotationRow[]>([]);

  // Which panel fills the right pane.
  const [rightView, setRightView] = useState<RightView>("recording");

  // Pass-click → spectrogram zoom+highlight.
  const [highlightRange, setHighlightRange] = useState<{ t0: number; t1: number } | null>(null);

  // Pass-hover → spectrogram green band.
  const [hoverRange, setHoverRange] = useState<{ t0: number; t1: number } | null>(null);

  // Audio playback cursor (real seconds in the recording).
  const [playbackPos, setPlaybackPos] = useState<number | null>(null);

  // Ref to the AudioPlayer so row-clicks can call seek() imperatively.
  const audioRef = useRef<AudioPlayerHandle>(null);

  // Pane sizes
  const [listWidth, setListWidth]               = useState(220);
  const [spectrogramHeight, setSpectrogramHeight] = useState(480);

  const onDragList       = useCallback((d: number) => setListWidth(w  => Math.max(140, Math.min(480, w + d))), []);
  const onDragSpectrogram = useCallback((d: number) => setSpectrogramHeight(h => Math.max(120, Math.min(900, h + d))), []);

  // ── Actions ──────────────────────────────────────────────────────────────────

  const refreshAnnotations = useCallback(async (dir: string) => {
    try {
      setAllAnnotations(await loadAnnotations(dir));
    } catch {
      // non-fatal: annotations may not exist yet
    }
  }, []);

  const handleOpenFolder = useCallback(async () => {
    const dir = await pickFolder();
    if (!dir) return;
    setError(null);
    setProjectDir(dir);
    setSelectedPath(null);
    setPayload(null);
    setRightView("recording");
    try {
      const [items] = await Promise.all([
        listRecordings(dir),
        refreshAnnotations(dir),
      ]);
      setRecordings(items);
      if (items.length === 0) setError("No WAV files found in that folder.");
    } catch (e) {
      setError(String(e));
    }
  }, [refreshAnnotations]);

  const handleSelectRecording = useCallback(
    async (item: RecordingListItem) => {
      if (analysing) return;
      setSelectedPath(item.path);
      setPayload(null);
      setError(null);
      setAnalysing(true);
      setRightView("recording");
      try {
        const result = await analyzeRecording(item.path, DEFAULT_THRESHOLD, DEFAULT_RATIO);
        setPayload(result);
      } catch (e) {
        setError(String(e));
      } finally {
        setAnalysing(false);
      }
    },
    [analysing]
  );

  const handleSave = useCallback(
    async (edits: AnnotationRow[]) => {
      if (!projectDir || saving) return;
      setSaving(true);
      try {
        await saveAnnotations(projectDir, edits);
        await refreshAnnotations(projectDir);
      } catch (e) {
        setError(String(e));
      } finally {
        setSaving(false);
      }
    },
    [projectDir, saving, refreshAnnotations]
  );

  // ── Derived ───────────────────────────────────────────────────────────────────

  const recordingId = payload?.file_name ?? selectedPath?.split("/").pop() ?? null;

  // Annotations saved for the currently loaded recording.
  // Memoised so PassGrid's useEffect([passes, savedAnnotations]) doesn't reset
  // row edits on every hover/hoverRange state change in App.
  const recordingAnnotations = useMemo(
    () => allAnnotations.filter((a) => a.recording_id === recordingId),
    [allAnnotations, recordingId]
  );

  // ── Layout ────────────────────────────────────────────────────────────────────

  return (
    <div style={styles.root}>
      {/* Toolbar */}
      <div style={styles.toolbar}>
        <span style={styles.appTitle}>Bat Review</span>
        <button style={styles.openBtn} onClick={handleOpenFolder}>
          Open folder…
        </button>
        {projectDir && (
          <span style={styles.dirLabel} title={projectDir}>
            {projectDir}
          </span>
        )}
        {projectDir && (
          <button
            style={{
              ...styles.viewBtn,
              ...(rightView === "all-annotations" ? styles.viewBtnActive : {}),
            }}
            onClick={() => setRightView(v => v === "all-annotations" ? "recording" : "all-annotations")}
          >
            All annotations
            {allAnnotations.length > 0 && (
              <span style={styles.badge}>{allAnnotations.length}</span>
            )}
          </button>
        )}
        {analysing && <span style={styles.statusBadge}>analysing…</span>}
        {error && (
          <span style={styles.errorBadge} title={error}>
            ⚠ {error.length > 80 ? error.slice(0, 77) + "…" : error}
          </span>
        )}
      </div>

      {/* Body */}
      <div style={styles.body}>

        {/* Left — recording list */}
        <div style={{ width: listWidth, flexShrink: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <RecordingList
            recordings={recordings}
            selectedPath={selectedPath}
            loading={analysing}
            onSelect={handleSelectRecording}
          />
        </div>

        <Divider onDrag={onDragList} axis="col" />

        {/* Right — view switcher */}
        <div style={styles.rightArea}>
          {rightView === "all-annotations" ? (
            <AnnotationsView
              annotations={allAnnotations}
              onSave={handleSave}
              saving={saving}
            />
          ) : (
            <>
              <div style={{ height: spectrogramHeight, flexShrink: 0, overflow: "hidden", display: "flex", flexDirection: "column" }}>
                <div style={{ flex: 1, overflow: "hidden" }}>
                  <SpectrogramViewer
                    htmlPath={payload?.html_path ?? null}
                    loading={analysing}
                    highlightRange={highlightRange}
                    hoverRange={hoverRange}
                    playbackPos={playbackPos}
                  />
                </div>
                <AudioPlayer
                  ref={audioRef}
                  wavPath={selectedPath}
                  onPosition={setPlaybackPos}
                />
              </div>

              <Divider onDrag={onDragSpectrogram} axis="row" />

              <div style={{ flex: 1, overflow: "hidden", minHeight: 0 }}>
                <PassGrid
                  recordingId={recordingId}
                  passes={payload?.passes ?? []}
                  savedAnnotations={recordingAnnotations}
                  onSave={handleSave}
                  saving={saving}
                  onRowClick={(t0, t1) => {
                    setHighlightRange({ t0, t1 });
                    audioRef.current?.seek(t0);
                  }}
                  onRowHover={setHoverRange}
                />
              </div>
            </>
          )}
        </div>

      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100vh",
    overflow: "hidden",
    background: "#1a1a1a",
    color: "#ddd",
    userSelect: "none",
  },
  toolbar: {
    display: "flex",
    alignItems: "center",
    padding: "0 12px",
    height: 36,
    background: "#141414",
    borderBottom: "1px solid #333",
    gap: 10,
    flexShrink: 0,
    overflow: "hidden",
  },
  appTitle: {
    fontWeight: 700,
    fontSize: 13,
    color: "#4af",
    flexShrink: 0,
    letterSpacing: 0.5,
  },
  openBtn: {
    background: "#1a3a5a",
    color: "#8cf",
    border: "1px solid #2a5a8a",
    borderRadius: 3,
    padding: "3px 10px",
    fontSize: 12,
    cursor: "pointer",
    flexShrink: 0,
  },
  dirLabel: {
    fontSize: 11,
    color: "#555",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    flex: 1,
  },
  viewBtn: {
    background: "transparent",
    color: "#888",
    border: "1px solid #333",
    borderRadius: 3,
    padding: "3px 10px",
    fontSize: 11,
    cursor: "pointer",
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    gap: 6,
  },
  viewBtnActive: {
    background: "#1a3a5a",
    color: "#4af",
    borderColor: "#2a5a8a",
  },
  badge: {
    background: "#2a4a6a",
    color: "#8cf",
    borderRadius: 8,
    padding: "0 5px",
    fontSize: 10,
    lineHeight: "16px",
  },
  statusBadge: {
    background: "#2a4a2a",
    color: "#6d6",
    borderRadius: 3,
    padding: "2px 7px",
    fontSize: 11,
    flexShrink: 0,
  },
  errorBadge: {
    background: "#4a1a1a",
    color: "#f88",
    borderRadius: 3,
    padding: "2px 7px",
    fontSize: 11,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    maxWidth: 500,
  },
  body: {
    display: "flex",
    flex: 1,
    overflow: "hidden",
  },
  rightArea: {
    flex: 1,
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
    minWidth: 0,
  },
};
