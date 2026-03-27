import { useEffect, useRef } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

interface Props {
  htmlPath: string | null;
  loading: boolean;
  /** If set, zoom+highlight this time range in the spectrogram. */
  highlightRange?: { t0: number; t1: number } | null;
  /** Hover range from the pass grid — shown as a green dashed band. */
  hoverRange?: { t0: number; t1: number } | null;
  /** Current audio playback position in seconds (drives the cursor line). */
  playbackPos?: number | null;
}

/**
 * Loads the Rust-generated spectrogram HTML from a local file path.
 * `convertFileSrc` converts the path to the `asset://` scheme that
 * Tauri's webview can serve directly — no IPC transfer of large HTML blobs.
 *
 * Communication with the iframe uses `postMessage` (the iframe has
 * `sandbox="allow-scripts"` but no `allow-same-origin`, so DOM access is
 * not available — postMessage is the correct cross-origin channel).
 */
export default function SpectrogramViewer({
  htmlPath,
  loading,
  highlightRange,
  hoverRange,
  playbackPos,
}: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const readyRef  = useRef(false);   // true once the iframe has fired its load event

  // (Re-)load the HTML whenever the recording changes.
  useEffect(() => {
    if (!iframeRef.current) return;
    readyRef.current = false;
    if (!htmlPath) {
      iframeRef.current.src = "about:blank";
      return;
    }
    const t0 = performance.now();
    iframeRef.current.src = convertFileSrc(htmlPath);
    const iframe = iframeRef.current;
    const onLoad = () => {
      readyRef.current = true;
      console.log(
        `[bat-review] iframe loaded in ${(performance.now() - t0).toFixed(0)} ms — ${htmlPath}`
      );
    };
    iframe.addEventListener("load", onLoad, { once: true });
    return () => iframe.removeEventListener("load", onLoad);
  }, [htmlPath]);

  // Forward highlight range to the iframe when it changes.
  useEffect(() => {
    if (!highlightRange || !iframeRef.current?.contentWindow) return;
    iframeRef.current.contentWindow.postMessage(
      { type: "zoomTo", t0: highlightRange.t0, t1: highlightRange.t1 },
      "*"
    );
  }, [highlightRange]);

  // Forward hover range (green dashed band, no zoom).
  useEffect(() => {
    if (!iframeRef.current?.contentWindow) return;
    if (hoverRange) {
      iframeRef.current.contentWindow.postMessage(
        { type: "hover", t0: hoverRange.t0, t1: hoverRange.t1 },
        "*"
      );
    } else {
      iframeRef.current.contentWindow.postMessage({ type: "hover", clear: true }, "*");
    }
  }, [hoverRange]);

  // Forward audio cursor position (throttled by RAF in the caller).
  useEffect(() => {
    if (playbackPos == null || !iframeRef.current?.contentWindow) return;
    iframeRef.current.contentWindow.postMessage(
      { type: "cursor", t: playbackPos },
      "*"
    );
  }, [playbackPos]);

  if (loading) {
    return <div style={styles.loading}>Analysing…</div>;
  }

  if (!htmlPath) {
    return (
      <div style={styles.placeholder}>
        Select a recording from the list to view its spectrogram.
      </div>
    );
  }

  return (
    <iframe
      ref={iframeRef}
      style={styles.iframe}
      sandbox="allow-scripts"
      title="Spectrogram viewer"
    />
  );
}

const styles: Record<string, React.CSSProperties> = {
  placeholder: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    height: "100%",
    background: "#111",
    color: "#444",
    fontSize: 13,
  },
  loading: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    height: "100%",
    background: "#111",
    color: "#666",
    fontSize: 13,
    fontStyle: "italic",
  },
  iframe: {
    width: "100%",
    height: "100%",
    border: "none",
    display: "block",
    background: "#111",
  },
};
