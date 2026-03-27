import type { RecordingListItem } from "../types";

interface Props {
  recordings: RecordingListItem[];
  selectedPath: string | null;
  loading: boolean;
  onSelect: (item: RecordingListItem) => void;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function RecordingList({
  recordings,
  selectedPath,
  loading,
  onSelect,
}: Props) {
  return (
    <div style={styles.root}>
      <div style={styles.header}>
        Recordings
        {loading && <span style={styles.badge}>analysing…</span>}
      </div>
      {recordings.length === 0 ? (
        <div style={styles.empty}>No WAV files found.<br />Open a folder above.</div>
      ) : (
        <ul style={styles.list}>
          {recordings.map((r) => (
            <li
              key={r.path}
              style={{
                ...styles.item,
                ...(r.path === selectedPath ? styles.selected : {}),
              }}
              onClick={() => onSelect(r)}
            >
              <span style={styles.name}>{r.file_name}</span>
              <span style={styles.size}>{formatSize(r.size_bytes)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    background: "#1e1e1e",
    borderRight: "1px solid #333",
    overflow: "hidden",
  },
  header: {
    padding: "8px 12px",
    fontSize: 11,
    fontWeight: 600,
    color: "#888",
    textTransform: "uppercase",
    letterSpacing: 1,
    borderBottom: "1px solid #333",
    display: "flex",
    alignItems: "center",
    gap: 8,
    flexShrink: 0,
  },
  badge: {
    background: "#2a4a2a",
    color: "#6d6",
    borderRadius: 3,
    padding: "1px 5px",
    fontSize: 10,
    fontWeight: 400,
    textTransform: "none",
    letterSpacing: 0,
  },
  list: {
    listStyle: "none",
    overflowY: "auto",
    flex: 1,
    margin: 0,
    padding: 0,
  },
  item: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "center",
    padding: "7px 12px",
    cursor: "pointer",
    borderBottom: "1px solid #2a2a2a",
    transition: "background 0.1s",
  },
  selected: {
    background: "#1a3a5a",
    boxShadow: "inset 0 0 0 2px #4af",
  },
  name: {
    fontSize: 12,
    color: "#ccc",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    flex: 1,
    marginRight: 8,
  },
  size: {
    fontSize: 10,
    color: "#555",
    flexShrink: 0,
  },
  empty: {
    padding: 16,
    fontSize: 12,
    color: "#555",
    textAlign: "center",
    lineHeight: 1.6,
  },
};
