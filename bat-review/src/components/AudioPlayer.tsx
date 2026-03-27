import {
  useState, useEffect, useRef, useCallback,
  forwardRef, useImperativeHandle,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

const RATES = [
  { label: "×0.05 (20× slower)", value: 0.05 },
  { label: "×0.1  (10× slower)", value: 0.1  },
  { label: "×0.25  (4× slower)", value: 0.25 },
  { label: "×0.5   (2× slower)", value: 0.5  },
  { label: "×1     (real time)", value: 1.0  },
];

export interface AudioPlayerHandle {
  /** Seek to `t` seconds (real recording time). Resumes if playing. */
  seek(t: number): void;
}

interface Props {
  wavPath: string | null;
  /** Called with current real-time position at ~10 Hz, or null when stopped. */
  onPosition?: (t: number | null) => void;
}

function fmt(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${(s % 60).toFixed(1).padStart(4, "0")}`;
}

const AudioPlayer = forwardRef<AudioPlayerHandle, Props>(
  function AudioPlayer({ wavPath, onPosition }, ref) {
    const [status, setStatus]     = useState<"idle" | "loading" | "ready" | "error">("idle");
    const [playing, setPlaying]   = useState(false);
    const [rate, setRate]         = useState(0.1);
    const [pos, setPos]           = useState(0);
    const [duration, setDuration] = useState(0);

    const ctxRef      = useRef<AudioContext | null>(null);
    const bufRef      = useRef<AudioBuffer | null>(null);
    const srcRef      = useRef<AudioBufferSourceNode | null>(null);
    const startCtxRef = useRef(0);   // ctx.currentTime when latest play started
    const startOfsRef = useRef(0);   // buffer offset (real seconds) at that start
    const rateRef     = useRef(rate);
    const rafRef      = useRef(0);

    rateRef.current = rate;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /** Current real-time position in the buffer (seconds). */
    const currentOffset = useCallback((): number => {
      if (!ctxRef.current || !srcRef.current) return startOfsRef.current;
      const elapsed = (ctxRef.current.currentTime - startCtxRef.current) * rateRef.current;
      return Math.min(startOfsRef.current + elapsed, bufRef.current?.duration ?? 0);
    }, []);

    /**
     * Stop the current source node.
     * IMPORTANT: null out `onended` first so the natural-end handler
     * (which resets startOfsRef to 0) does not fire on a manual stop.
     */
    const stopSource = useCallback(() => {
      cancelAnimationFrame(rafRef.current);
      if (srcRef.current) {
        srcRef.current.onended = null;        // ← prevents position reset on pause
        try { srcRef.current.stop(); } catch { /* already stopped */ }
        srcRef.current = null;
      }
    }, []);

    const startPlayback = useCallback((offset: number, r: number) => {
      if (!ctxRef.current || !bufRef.current) return;
      const ctx = ctxRef.current;
      stopSource();

      const src = ctx.createBufferSource();
      src.buffer = bufRef.current;
      src.playbackRate.value = r;
      src.connect(ctx.destination);
      src.start(0, Math.max(0, offset));

      startCtxRef.current = ctx.currentTime;
      startOfsRef.current = offset;
      srcRef.current = src;
      setPlaying(true);

      // Only fires on natural end (onended is nulled before manual stops).
      src.onended = () => {
        cancelAnimationFrame(rafRef.current);
        startOfsRef.current = 0;
        setPos(0);
        setPlaying(false);
        onPosition?.(null);
      };

      // Position tracker ~10 fps.
      let lastReport = -1;
      const tick = () => {
        if (!srcRef.current) return;
        const p = currentOffset();
        setPos(p);
        const rounded = Math.round(p * 10) / 10;
        if (rounded !== lastReport) { onPosition?.(p); lastReport = rounded; }
        rafRef.current = requestAnimationFrame(tick);
      };
      rafRef.current = requestAnimationFrame(tick);
    }, [stopSource, currentOffset, onPosition]);

    // ── Load new WAV ──────────────────────────────────────────────────────────

    useEffect(() => {
      stopSource();
      setStatus("idle");
      setPlaying(false);
      setPos(0);
      setDuration(0);
      startOfsRef.current = 0;
      bufRef.current = null;
      onPosition?.(null);

      if (!wavPath) return;
      setStatus("loading");

      if (!ctxRef.current) ctxRef.current = new AudioContext();
      const ctx = ctxRef.current;

      let cancelled = false;
      fetch(convertFileSrc(wavPath))
        .then(r => r.arrayBuffer())
        .then(ab => ctx.decodeAudioData(ab))
        .then(buf => {
          if (cancelled) return;
          bufRef.current = buf;
          setDuration(buf.duration);
          setStatus("ready");
        })
        .catch(err => {
          if (cancelled) return;
          console.error("[bat-review] audio decode error:", err);
          setStatus("error");
        });

      return () => { cancelled = true; stopSource(); };
    }, [wavPath, stopSource, onPosition]);

    // ── Imperative handle — lets App.tsx call seek() on a row click ──────────

    useImperativeHandle(ref, () => ({
      seek(t: number) {
        const clamped = Math.max(0, Math.min(t, bufRef.current?.duration ?? 0));
        startOfsRef.current = clamped;
        setPos(clamped);
        onPosition?.(clamped);
        // If currently playing, restart at new position with current rate.
        if (srcRef.current) startPlayback(clamped, rateRef.current);
      },
    }), [startPlayback, onPosition]);

    // ── Controls ─────────────────────────────────────────────────────────────

    const handleToggle = useCallback(() => {
      if (playing) {
        startOfsRef.current = currentOffset();   // save position before stopping
        stopSource();
        setPlaying(false);
        onPosition?.(null);
      } else {
        startPlayback(startOfsRef.current, rate);
      }
    }, [playing, rate, currentOffset, stopSource, startPlayback, onPosition]);

    const handleRateChange = useCallback((newRate: number) => {
      setRate(newRate);
      if (playing) {
        startPlayback(currentOffset(), newRate);
      }
    }, [playing, currentOffset, startPlayback]);

    const handleSeek = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
      const t = parseFloat(e.target.value);
      startOfsRef.current = t;
      setPos(t);
      onPosition?.(t);
      if (playing) startPlayback(t, rate);
    }, [playing, rate, startPlayback, onPosition]);

    // ── Render ────────────────────────────────────────────────────────────────

    if (!wavPath) return null;
    const ready = status === "ready";

    return (
      <div style={styles.bar}>
        <button
          style={styles.playBtn}
          onClick={handleToggle}
          disabled={!ready}
          title={playing ? "Pause" : "Play"}
        >
          {playing ? "⏸" : "▶"}
        </button>

        <span style={styles.time}>{ready ? fmt(pos) : "--:--.--"}</span>

        <input
          type="range"
          min={0}
          max={duration || 1}
          step={0.05}
          value={pos}
          onChange={handleSeek}
          disabled={!ready}
          style={styles.seek}
        />

        <span style={styles.time}>{ready ? fmt(duration) : "--:--.--"}</span>

        <select
          value={rate}
          onChange={e => handleRateChange(parseFloat(e.target.value))}
          style={styles.select}
          disabled={!ready}
        >
          {RATES.map(r => (
            <option key={r.value} value={r.value}>{r.label}</option>
          ))}
        </select>

        {status === "loading" && <span style={styles.hint}>Loading audio…</span>}
        {status === "error"   && <span style={{ ...styles.hint, color: "#f88" }}>Audio decode failed</span>}
        {status === "ready" && !playing && pos === 0 && (
          <span style={styles.hint}>
            Bat calls pitch-shifted down on decode — ×0.1 recommended
          </span>
        )}
      </div>
    );
  }
);

export default AudioPlayer;

const styles: Record<string, React.CSSProperties> = {
  bar: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "0 10px",
    height: 40,
    background: "#141414",
    borderTop: "1px solid #2a2a2a",
    flexShrink: 0,
    overflow: "hidden",
  },
  playBtn: {
    background: "#1a3a1a",
    color: "#8d8",
    border: "1px solid #2a5a2a",
    borderRadius: 3,
    width: 28,
    height: 24,
    fontSize: 12,
    cursor: "pointer",
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  time: {
    fontSize: 11,
    color: "#666",
    fontFamily: "monospace",
    flexShrink: 0,
    minWidth: 52,
  },
  seek: {
    flex: 1,
    minWidth: 60,
    accentColor: "#4af",
    cursor: "pointer",
  },
  select: {
    background: "#1e1e1e",
    color: "#aaa",
    border: "1px solid #333",
    borderRadius: 3,
    fontSize: 11,
    padding: "2px 4px",
    cursor: "pointer",
    flexShrink: 0,
  },
  hint: {
    fontSize: 10,
    color: "#444",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    flex: 1,
    minWidth: 0,
  },
};
