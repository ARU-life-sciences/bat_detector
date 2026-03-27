# bat_detector — CLI tool and Bat Review GUI

A command-line bat call detector and classifier for AudioMoth WAV recordings. Processes ultrasonic WAV files, identifies British bat species from their echolocation calls, and produces spectrograms and a detection CSV for survey analysis.

---

## Bat Review — desktop GUI

Bat Review is a desktop app for reviewing AudioMoth WAV recordings analysed by the CLI tool.  It shows an interactive spectrogram, lists the detected bat passes, lets you play back the recording at slowed-down speed, and stores your review notes in a CSV file alongside the recordings.

### Installation

Download the latest release for your platform from the [Releases page](../../releases):

- **macOS Apple Silicon** — `.dmg` (arm64)
- **macOS Intel** — `.dmg` (x86_64)
- **Windows** — `.msi` installer
- **Linux** — `.AppImage`

On macOS you may need to right-click → Open the first time to bypass Gatekeeper.

### Quick start

1. Launch **Bat Review**.
2. Click **Open folder…** and select the directory containing your `.WAV` files.  The app lists all WAV files found; recordings that already have a cached analysis load instantly.
3. Click any recording in the left panel to analyse it.  The spectrogram appears in the top pane and the detected passes appear in the table below.

### Layout

```
┌─────────────────────────────────────────────────────────────┐
│  Toolbar: Open folder  |  folder path  |  All annotations   │
├──────────┬──────────────────────────────────────────────────┤
│          │  Spectrogram viewer                              │
│Recording │  ──────────────────────── (drag divider)         │
│  list    │  Audio player                                    │
│          │  ══════════════════════════════════════════════  │
│          │  Pass table                                      │
└──────────┴──────────────────────────────────────────────────┘
```

All dividers are draggable — grab the thin bar between panels and drag to resize.

### Spectrogram viewer

| Control | Action |
|---------|--------|
| Scroll wheel | Zoom in/out on the time axis |
| Shift + scroll | Zoom in/out on the frequency axis |
| Click + drag | Pan |
| Shift + drag | Draw a selection (blue band) — filters the pass table to overlapping passes |
| Escape | Clear selection |
| Double-click | Reset to full view |
| Click a pass row (table) | Zoom to that pass and seek audio |
| Hover a pass row (table) | Green band shows the pass time range |

The **red strip** at the top marks windows with detected bat energy.  **White vertical lines** mark call-group boundaries.  **Dashed white lines** mark the 20 kHz and 120 kHz band limits.

### Audio player

The audio player bar sits below the spectrogram.  AudioMoth recordings at 250 kHz are decoded by the browser's 44.1 kHz audio engine, which automatically time-expands and pitch-shifts the calls into the audible range.

| Control | Action |
|---------|--------|
| Play / Pause | Start or pause playback |
| Speed selector | `×0.05` – `×1.0` — slow down further for faint or fast calls |
| Click a pass row | Seeks audio to the start of that pass |

An **amber vertical line** tracks the playback position on the spectrogram in real time.

### Pass table

Each row is one detected bat pass.  Read-only columns (from the analysis):

| Column | Description |
|--------|-------------|
| # | Pass index within the recording |
| Time | Start – end time (seconds) |
| Dur | Duration (seconds) |
| Pulses | Number of echolocation pulses detected |
| Peak kHz | Mean peak frequency |
| Code | Six-letter species code (e.g. `PIPPIP`) |
| Species | Full species name |
| Conf | Confidence badge — green ≥ 75 %, amber ≥ 40 %, red < 40 % |
| Dubious | Flagged when a single-pulse pass is nested inside a larger pass of a different species |

Editable columns (highlighted in blue, click to edit):

| Column | Description |
|--------|-------------|
| ★ Rev. code | Your reviewed species code |
| ★ Rev. species | Your reviewed species name |
| ★ Keep | Checkbox — uncheck to exclude from export |
| ★ Notes | Free-text notes |

### Saving your review

Click **Save review** (bottom-right of the pass table) to write your annotations for the current recording.  Annotations are stored in `review/annotations.csv` inside the folder you opened — one row per pass, across all recordings.  The file is created automatically and is safe to open in Excel or R.

Switching to a different recording before saving will lose unsaved edits, so save frequently.

### All annotations view

Click the **All annotations** button in the toolbar to see every saved annotation across all recordings in a single sortable table.  You can edit the reviewed fields here too and click **Save all** to commit.  Click the button again (or select a recording) to return to the per-recording view.

### Annotations CSV columns

| Column | Description |
|--------|-------------|
| `recording_id` | WAV filename |
| `pass_idx` | Pass index within the recording |
| `start_sec`, `end_sec` | Pass time boundaries |
| `n_pulses` | Pulses detected |
| `n_extra` | Sub-threshold nearby pulses |
| `mean_peak_khz` | Mean peak frequency (kHz) |
| `freq_low_khz`, `freq_high_khz` | −20 dB frequency bounds |
| `bandwidth_khz` | −10 dB bandwidth |
| `rep_rate` | Pulse repetition rate (pulses/s) |
| `call_dur_ms` | Mean pulse duration (ms) |
| `mean_energy_db` | Mean bat-band energy |
| `confidence` | Classifier confidence (0–1) |
| `auto_code`, `auto_species` | Classifier output |
| `is_cf` | `true` for constant-frequency (horseshoe bat) calls |
| `dubious` | Classifier quality flag |
| `reviewed_code`, `reviewed_species` | Your reviewed identification |
| `keep` | Whether to include this pass in exports |
| `notes` | Your review notes |
| `updated_at` | Timestamp of last save |

---

## CLI tool

## Build

Requires [Rust](https://rustup.rs/).

```
cargo build --release
```

The binary is at `target/release/bat_detector`.

## Usage

```
bat_detector [--output] [--threshold <n>] <file.wav | directory>
```

- By default, output files are only written when bat calls are detected.
- `--output` forces output even when nothing is detected.
- `--threshold <n>` overrides the detection sensitivity (default `3.0`).  The detector flags a window when its bat-band energy exceeds the local noise floor by this factor.  Raise the value (e.g. `--threshold 5.0` or `--threshold 8.0`) in recordings with heavy low-frequency interference (traffic, wind) to reduce false positives; lower it to catch faint or distant calls.

**Single file:**

```bash
bat_detector data/20260322_190000.WAV
```

**Batch — process an entire directory:**

```bash
bat_detector data/
```

All WAV files in the directory are processed in filename order.  Per-file outputs (PNG, HTML, CSV) are written as usual, and a combined `survey.csv` is written into the directory alongside a terminal species summary:

```
── Batch summary ──────────────────────────────────────────────
  Files processed : 42  (17 with bat activity)

  Code      Species                                 Passes  Pulses
  ────────────────────────────────────────────────────────────────
  PIPPIP    Common pipistrelle (Pipistrellus pip…)     38     412
  PIPPYG    Soprano pipistrelle (Pipistrellus py…)     21     198
  MYODAU    Daubenton's myotis (Myotis daubenton…)      4      17
───────────────────────────────────────────────────────────────
```

Dubious passes are excluded from the summary counts.

## Output files

For an input file `data/20260322_190000.WAV` the following are written alongside it:

| File | Description |
|------|-------------|
| `…_spectrogram.png` | Static spectrogram image with detection markers |
| `…_spectrogram.html` | Interactive spectrogram viewer (self-contained) |
| `…_detections.csv` | One row per species pass (append across files for survey analysis) |

The date and time are parsed from the filename when it follows the AudioMoth convention `YYYYMMDD_HHMMSS.WAV`.

## Interactive HTML viewer

Open the HTML file in any browser — no server required.

| Control | Action |
|---------|--------|
| Scroll | Zoom in/out on the time axis |
| Shift + scroll | Zoom in/out on the frequency axis |
| Drag | Pan |
| Shift + drag | Select a time range — filters the passes table to matching species |
| Escape | Clear selection and restore all table rows |
| Double-click | Reset view and clear selection |
| Click a pass row | Zoom to that pass |
| Mouse hover | Crosshair with time, frequency, intensity (dB), and species label |

The red strip along the top of the spectrogram marks windows where bat energy was detected. White vertical lines mark the boundaries of call groups. Dashed white lines mark the 20 kHz and 120 kHz band limits.

## CSV columns

| Column | Description |
|--------|-------------|
| `filename` | Source WAV filename |
| `date` | Date parsed from filename (`YYYY-MM-DD`) |
| `time` | Time parsed from filename (`HH:MM:SS`) |
| `pass` | Pass index within this file |
| `start_s`, `end_s` | Pass start and end time (seconds) |
| `duration_ms` | Pass duration (milliseconds) |
| `n_pulses` | Pulses detected above threshold |
| `n_extra` | Sub-threshold pulses found by the local ±1 s search (single-pulse passes only) |
| `mean_peak_khz` | Mean peak frequency across pulses in the pass (kHz) |
| `peak_hz_std_khz` | Standard deviation of peak frequency across pulses (kHz); zero for single-pulse passes; low values indicate a tightly-clustered, species-consistent call sequence |
| `freq_low_khz` | Mean −20 dB lower frequency bound (kHz) |
| `freq_high_khz` | Mean −20 dB upper frequency bound (kHz) |
| `bandwidth_khz` | Mean −10 dB bandwidth (kHz) |
| `cf_tail_ratio` | Energy concentration at peak (0–1); high values indicate an FM+CF call shape |
| `rep_rate_hz` | Mean pulse repetition rate (pulses/s) |
| `is_cf` | `true` for narrowband constant-frequency calls (horseshoe bats) |
| `mean_energy_db` | Mean bat-band power across detected windows (dB re FFT² units) |
| `peak_energy_db` | Peak bat-band power in any single detected window (dB re FFT² units) |
| `code` | Six-letter species code (e.g. `PIPPYG`) |
| `species` | Species common and scientific name |
| `notes` | Diagnostic notes from the classifier (field characteristics, potential confusers) |
| `dubious` | `true` when a single-pulse pass is nested inside a larger pass of a different species |
| `confidence` | Identification confidence score (0–1); see below |

### Confidence score

The `confidence` column is a 0–1 score summarising how reliable the identification is.  It is the product of two components:

**Pulse-count score** — rises from 0 towards 1 as more pulses accumulate, using `1 − exp(−n / 3)`:

| Effective pulses | Score |
|-----------------|-------|
| 1 | 0.28 |
| 2 | 0.49 |
| 3 | 0.63 |
| 5 | 0.81 |
| 10 | 0.97 |

For single-pulse passes, `n_extra` sub-threshold nearby pulses are added to the count before scoring, so an isolated bat in an otherwise active area receives a higher score than a truly isolated single click.

**Frequency-consistency score** — coefficient of variation of `mean_peak_khz` across pulses, scaled so a CV of 10 % gives 0.5 and 0 % gives 1.0.  Single-pulse passes (where no variance can be measured) are given a consistency of 1.0.

Passes flagged `dubious` always receive a confidence of 0.  The HTML viewer colour-codes the score: green ≥ 75 %, amber ≥ 40 %, red below 40 %.

### Using energy for relative distance

`mean_energy_db` and `peak_energy_db` are in units of `10 · log₁₀(FFT amplitude²)`.  They are **not** calibrated to dB SPL but are directly comparable across recordings made with the same AudioMoth and gain setting.  A louder value indicates a closer or louder bat.  Sound intensity falls as `1/r²` in open air, so a 6 dB difference corresponds to roughly a factor of 2 in distance.

To estimate relative activity, group the CSV by species and date, count rows (`n_pulses`), and compare `mean_energy_db` distributions.

## Detection method

1. The WAV is divided into non-overlapping 1024-point Hann-windowed frames.
2. Each window must satisfy two independent conditions to be flagged as bat activity:
   - **Adaptive noise floor** — the mean bat-band (20–120 kHz) energy must exceed the 10th-percentile of bat-band energies in the surrounding ±3 seconds by a factor of 3×.  Using a low rolling percentile makes this robust to constant ultrasonic interference (insects, machinery) because the threshold rises and falls with the local background.
   - **Spectral ratio** — the bat-band mean must also exceed the whole-spectrum mean by a factor of at least 1.05.  Broadband noise sources such as traffic and wind elevate all frequency bins equally, keeping this ratio near 1.0, so they are rejected even if they pass the adaptive check.  Real bat calls concentrate energy in the bat band and comfortably clear this threshold.
3. Consecutive flagged windows (with gaps ≤ 25 windows filled) are grouped into call groups.
4. Spectral features are extracted from each group: peak frequency, bandwidth, frequency range, CF-tail energy ratio, and pulse repetition rate.
5. Calls are classified with the British bat key (Cornes, Bedfordshire Bat Group, 2008).  The Noctule path additionally requires a rep rate ≤ 8 /s; without this guard, low-frequency interference (traffic, wind) that happens to peak at 20–26 kHz and hits the 18 kHz frequency floor can satisfy the frequency condition and flood the Noctule bucket.
6. Consecutive same-species groups within 2 s of each other are merged into a single *pass*.
7. Single-pulse passes nested inside a larger pass are flagged as dubious.
8. For remaining single-pulse passes, a local search ±1 s around the pulse looks for sub-threshold pulses at the same frequency to estimate isolation.

## Species codes

The following codes are used in the `species` column and on-canvas labels.  Species marked † are outside the normal British range and would require verification.

| Code | Scientific name | Common name |
|------|----------------|-------------|
| BARBAR | *Barbastella barbastellus* | Western barbastelle |
| EPTSER | *Eptesicus serotinus* | Common serotine |
| MYOBEC | *Myotis bechsteinii* | Bechstein's myotis |
| MYOBRA | *Myotis brandtii* | Brandt's myotis |
| MYODAU | *Myotis daubentonii* | Daubenton's myotis |
| MYOMYS | *Myotis mystacinus* | Whiskered myotis |
| MYONAT | *Myotis nattereri* | Natterer's myotis |
| MYOSPP | *Myotis* sp. | Unresolved / probable Daubenton's, Whiskered or Brandt's |
| NYCLEI | *Nyctalus leisleri* | Leisler's noctule |
| NYCNOC | *Nyctalus noctula* | Noctule |
| NYCSPP | *Nyctalus* sp. | Noctule or Leisler's (ambiguous floor 21–24 kHz) |
| PIPNAT | *Pipistrellus nathusii* | Nathusius' pipistrelle |
| PIPPIP | *Pipistrellus pipistrellus* | Common pipistrelle |
| PIPPYG | *Pipistrellus pygmaeus* | Soprano pipistrelle |
| PIPSPP | *Pipistrellus* sp. | Common or Nathusius' pipistrelle (boundary) |
| PLEAUR | *Plecotus auritus* | Brown long-eared bat |
| PLEAUS | *Plecotus austriacus* | Grey long-eared bat |
| PLESPP | *Plecotus* sp. | Unresolved long-eared bat |
| RHIFER | *Rhinolophus ferrumequinum* | Greater horseshoe bat |
| RHIHIP | *Rhinolophus hipposideros* | Lesser horseshoe bat |
| RHISPP | *Rhinolophus* sp. | Unresolved horseshoe bat |

Additional European species recognised by the classifier but not currently output (would require expanded detection logic):

| Code | Scientific name | Common name |
|------|----------------|-------------|
| EPTISA† | *Eptesicus isabellinus* | Meridional serotine |
| EPTNIL† | *Eptesicus nilssonii* | Northern bat |
| HYPSAV† | *Hypsugo savii* | Savi's pipistrelle |
| MINSCH† | *Miniopterus schreibersii* | Common bent-wing bat |
| MYOALC | *Myotis alcathoe* | Alcathoe myotis |
| MYOCAP† | *Myotis capaccinii* | Long-fingered bat |
| MYODAS† | *Myotis dasycneme* | Pond myotis |
| MYOEMA† | *Myotis emarginatus* | Geoffroy's myotis |
| MYOMYO† | *Myotis myotis* | Mouse-eared myotis |
| NYCLAS† | *Nyctalus lasiopterus* | Giant noctule |
| PIPKUH† | *Pipistrellus kuhlii* | Kuhl's pipistrelle |
| RHIEUR† | *Rhinolophus euryale* | Mediterranean horseshoe bat |
| TADTEN† | *Tadarida teniotis* | European free-tailed bat |
| VESMUR† | *Vespertilio murinus* | Particoloured bat |
