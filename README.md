# bat_detector

A command-line bat call detector and classifier for AudioMoth WAV recordings. Processes ultrasonic WAV files, identifies British bat species from their echolocation calls, and produces spectrograms and a detection CSV for survey analysis.

## Build

Requires [Rust](https://rustup.rs/).

```
cargo build --release
```

The binary is at `target/release/bat_detector`.

## Usage

```
bat_detector [--output] <file.wav>
```

- By default, output files are only written when bat calls are detected.
- `--output` forces output even when nothing is detected (useful for batch processing null results).

**Batch processing:**

```bash
for f in data/*.WAV; do bat_detector "$f"; done
```

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

### Using energy for relative distance

`mean_energy_db` and `peak_energy_db` are in units of `10 · log₁₀(FFT amplitude²)`.  They are **not** calibrated to dB SPL but are directly comparable across recordings made with the same AudioMoth and gain setting.  A louder value indicates a closer or louder bat.  Sound intensity falls as `1/r²` in open air, so a 6 dB difference corresponds to roughly a factor of 2 in distance.

To estimate relative activity, group the CSV by species and date, count rows (`n_pulses`), and compare `mean_energy_db` distributions.

## Detection method

1. The WAV is divided into non-overlapping 1024-point Hann-windowed frames.
2. For each window the mean power in the 20–120 kHz bat band is computed.  A rolling 10th-percentile of bat-band energy over the surrounding ±3 seconds provides a local noise floor estimate.  A window is flagged as bat activity when its bat-band energy exceeds that noise floor by a factor of 3×.  Using a low percentile of the local neighbourhood (rather than the current frame's whole-spectrum mean) makes detection robust to constant ultrasonic interference sources such as insects or machinery, because the threshold rises and falls with the background.
3. Consecutive flagged windows (with gaps ≤ 25 windows filled) are grouped into call groups.
4. Spectral features are extracted from each group: peak frequency, bandwidth, frequency range, CF-tail energy ratio, and pulse repetition rate.
5. Calls are classified with the British bat key (Cornes, Bedfordshire Bat Group, 2008).
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
