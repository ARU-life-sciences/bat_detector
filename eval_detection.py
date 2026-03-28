"""
Evaluate the bat detector against BatDetective ground-truth call positions.

For each test WAV the ground truth gives a list of call timestamps (seconds).
We run a simple energy detector in the appropriate frequency band (adjusted for
time-expansion) and compare detections against ground truth using a ±200 ms
tolerance window, reporting per-set precision / recall / F1.

Usage:
    python eval_detection.py [--tolerance 0.2] [--sets uk norfolk bulgaria]

The script auto-detects the time-expansion factor from the WAV sample rate:
    38 400 Hz → 10× expansion (original 384 kHz) → bat band 2 000–12 000 Hz
    44 100 Hz → 5.67× expansion (original 250 kHz) → bat band 3 500–21 000 Hz
"""

import argparse
import os
import wave

import numpy as np


# ── Constants ────────────────────────────────────────────────────────────────

DATA_DIR   = os.path.dirname(os.path.abspath(__file__))
WAV_DIR    = os.path.join(DATA_DIR, "training_data", "wav")
SPLIT_DIR  = os.path.join(DATA_DIR, "training_data", "train_test_split")

# Time-expansion factor → bat frequency band in the expanded audio (Hz)
BAND_FOR_RATE = {
    38400: (2_000, 12_000),   # 10× expanded from 384 kHz
    44100: (3_500, 21_000),   # 5.67× expanded from 250 kHz
}

# Detection parameters (mirroring the Rust detector, adapted for expanded audio)
FFT_WIN      = 512    # samples per FFT frame (shorter → better time resolution)
NOISE_PERC   = 10     # adaptive floor percentile (10th = same as Rust code)
NOISE_NSEC   = 2.0    # neighbourhood radius (seconds) for noise floor estimate
SNR_THRESH   = 3.0    # signal / noise floor threshold (same as CLI default)
MERGE_GAP_S  = 0.15   # merge detections within this gap (s)
MIN_DUR_S    = 0.010  # minimum detection duration (s)


# ── Core detector ────────────────────────────────────────────────────────────

def load_wav_mono(path: str) -> tuple[np.ndarray, int]:
    with wave.open(path) as w:
        sr    = w.getframerate()
        n     = w.getnframes()
        raw   = w.readframes(n)
        depth = w.getsampwidth()
    dtype = {1: np.int8, 2: np.int16, 4: np.int32}[depth]
    samples = np.frombuffer(raw, dtype=dtype).astype(np.float32)
    if w.getnchannels() > 1:
        samples = samples.reshape(-1, w.getnchannels()).mean(axis=1)
    samples /= np.iinfo(dtype).max
    return samples, sr


def bat_band_energy(samples: np.ndarray, sr: int, win: int = FFT_WIN) -> np.ndarray:
    """Return mean bat-band power for each FFT frame (no overlap)."""
    lo, hi = BAND_FOR_RATE.get(sr, (3_500, 21_000))
    freqs   = np.fft.rfftfreq(win, 1.0 / sr)
    mask    = (freqs >= lo) & (freqs <= hi)

    n_frames = len(samples) // win
    energy   = np.empty(n_frames, dtype=np.float32)
    hann     = np.hanning(win)
    for i in range(n_frames):
        frame      = samples[i * win : (i + 1) * win] * hann
        spectrum   = np.abs(np.fft.rfft(frame)) ** 2
        energy[i]  = spectrum[mask].mean() if mask.any() else 0.0
    return energy


def detect_calls(samples: np.ndarray, sr: int) -> list[float]:
    """
    Return a list of call-centre timestamps (seconds) using an adaptive
    noise-floor detector, the same logic as the Rust code.
    """
    energy    = bat_band_energy(samples, sr)
    frame_dur = FFT_WIN / sr          # seconds per FFT frame
    n_frames  = len(energy)

    # Adaptive noise floor: 10th-percentile in a ±NOISE_NSEC neighbourhood
    half_win = max(1, int(NOISE_NSEC / frame_dur))
    floor    = np.empty_like(energy)
    for i in range(n_frames):
        lo       = max(0, i - half_win)
        hi       = min(n_frames, i + half_win + 1)
        floor[i] = np.percentile(energy[lo:hi], NOISE_PERC)

    detected = (floor > 0) & (energy > SNR_THRESH * floor)

    # Convert boolean flags → merged intervals → midpoint timestamps
    calls      = []
    in_call    = False
    call_start = 0
    for i, flag in enumerate(detected):
        t = i * frame_dur
        if flag and not in_call:
            in_call    = True
            call_start = t
        elif not flag and in_call:
            call_end  = t
            dur       = call_end - call_start
            if dur >= MIN_DUR_S:
                calls.append((call_start + call_end) / 2)
            in_call = False
    if in_call:
        call_end = n_frames * frame_dur
        if call_end - call_start >= MIN_DUR_S:
            calls.append((call_start + call_end) / 2)

    # Merge calls that are close together
    if len(calls) < 2:
        return calls
    merged = [calls[0]]
    for c in calls[1:]:
        if c - merged[-1] <= MERGE_GAP_S:
            merged[-1] = (merged[-1] + c) / 2  # average midpoint
        else:
            merged.append(c)
    return merged


# ── Evaluation ───────────────────────────────────────────────────────────────

def match_calls(detected: list[float], truth: list[float], tol: float) -> tuple[int, int, int]:
    """
    Greedy nearest-neighbour matching within ±tol seconds.
    Returns (tp, fp, fn).
    """
    truth_remaining = list(truth)
    tp = 0
    fp = 0
    for d in detected:
        candidates = [t for t in truth_remaining if abs(d - t) <= tol]
        if candidates:
            best = min(candidates, key=lambda t: abs(d - t))
            truth_remaining.remove(best)
            tp += 1
        else:
            fp += 1
    fn = len(truth_remaining)
    return tp, fp, fn


def evaluate_set(npz_path: str, wav_dir: str, tol: float) -> dict:
    data  = np.load(npz_path, allow_pickle=True, encoding="latin1")
    files = [f.decode() for f in data["test_files"]]
    poses = data["test_pos"]

    total_tp = total_fp = total_fn = 0
    n_files  = 0
    n_missing = 0
    errors    = []

    for fname, pos in zip(files, poses):
        wav_path = os.path.join(wav_dir, fname + ".wav")
        if not os.path.exists(wav_path):
            n_missing += 1
            continue

        truth = list(pos.flatten())
        try:
            samples, sr = load_wav_mono(wav_path)
            detected    = detect_calls(samples, sr)
        except Exception as e:
            errors.append(f"{fname}: {e}")
            continue

        tp, fp, fn = match_calls(detected, truth, tol)
        total_tp  += tp
        total_fp  += fp
        total_fn  += fn
        n_files   += 1

    precision = total_tp / (total_tp + total_fp) if (total_tp + total_fp) > 0 else 0.0
    recall    = total_tp / (total_tp + total_fn) if (total_tp + total_fn) > 0 else 0.0
    f1        = (2 * precision * recall / (precision + recall)
                 if (precision + recall) > 0 else 0.0)

    return {
        "files_evaluated": n_files,
        "files_missing":   n_missing,
        "tp": total_tp, "fp": total_fp, "fn": total_fn,
        "precision": precision, "recall": recall, "f1": f1,
        "errors": errors,
    }


# ── Main ─────────────────────────────────────────────────────────────────────

SET_PATHS = {
    "uk":       os.path.join(SPLIT_DIR, "test_set_uk.npz"),
    "norfolk":  os.path.join(SPLIT_DIR, "test_set_norfolk.npz"),
    "bulgaria": os.path.join(SPLIT_DIR, "test_set_bulgaria.npz"),
}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--tolerance", type=float, default=0.2,
                        help="Match tolerance in seconds (default 0.2)")
    parser.add_argument("--sets", nargs="+", default=["uk", "norfolk"],
                        choices=list(SET_PATHS.keys()))
    args = parser.parse_args()

    print(f"Tolerance: ±{args.tolerance} s\n")
    print(f"{'Set':<10} {'Files':>6} {'TP':>6} {'FP':>6} {'FN':>6} "
          f"{'Prec':>7} {'Rec':>7} {'F1':>7}")
    print("─" * 62)

    for set_name in args.sets:
        r = evaluate_set(SET_PATHS[set_name], WAV_DIR, args.tolerance)
        print(f"{set_name:<10} {r['files_evaluated']:>6} {r['tp']:>6} {r['fp']:>6} "
              f"{r['fn']:>6} {r['precision']:>7.3f} {r['recall']:>7.3f} {r['f1']:>7.3f}")
        if r["files_missing"]:
            print(f"  ({r['files_missing']} WAVs not found)")
        for e in r["errors"][:3]:
            print(f"  error: {e}")

    print()
    # BatDetective published results for reference (from paper Table 1)
    print("BatDetective published F1 (±200ms):")
    print("  UK test set:      0.824")
    print("  Norfolk test set: 0.690")
    print("  Bulgaria test set: 0.771")
