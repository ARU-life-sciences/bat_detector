// ── Colour map ────────────────────────────────────────────────────────────────
// Pre-compute a 256-entry heat palette (black→blue→cyan→green→yellow→red)
// stored as a flat Uint8ClampedArray of [R,G,B, R,G,B, ...].
// Looked up at render time: index v (0–255) → CMAP[v*3 .. v*3+2].
const CMAP = (function() {
  const t = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i++) {
    const v = i / 255; let r, g, b;
    if (v < .25) { const s = v * 4; r = 0; g = 0; b = 255 * s | 0; }
    else if (v < .5) { const s = (v - .25) * 4; r = 0; g = 255 * s | 0; b = 255; }
    else if (v < .75) { const s = (v - .5) * 4; r = 255 * s | 0; g = 255; b = 255 * (1 - s) | 0; }
    else { const s = (v - .75) * 4; r = 255; g = 255 * (1 - s) | 0; b = 0; }
    t[i * 3] = r; t[i * 3 + 1] = g; t[i * 3 + 2] = b;
  }
  return t;
})();

// ── Canvas setup ──────────────────────────────────────────────────────────────
// cv / ct  — the visible canvas the user sees.
// ov / oc  — an offscreen canvas used as a clean snapshot.
//
// Two-layer rendering strategy:
//   render()  → expensive pixel loop; draws spectrogram + axes + markers onto
//               cv, then copies the result to ov as a "clean state" snapshot.
//   repaint() → cheap; copies ov back to cv, then composites the selection
//               rectangle and crosshair on top.  Called on every mousemove so
//               the cursor overlays never require a full pixel loop.
const cv = document.getElementById('cv'); const ct = cv.getContext('2d');
const ov = document.createElement('canvas'); const oc = ov.getContext('2d');

// Layout constants (pixels):
//   MH  — height of the detection strip at the top of the spectrogram area.
//   AH  — height of the time axis below the spectrogram.
//   FAW — width of the frequency axis to the left of the spectrogram.
const MH = 10, AH = 24, FAW = 56;

// ── View state ────────────────────────────────────────────────────────────────
// view describes which slice of the data is currently visible.
//   x0/x1 — window indices (0–D.nW) on the time axis.
//   y0/y1 — bin indices    (0–D.nB) on the frequency axis.
// Zooming and panning update view, then call render().
let view = { x0: 0, x1: D.nW, y0: 0, y1: D.nB };

// sel      — the current time-range selection as {w0, w1} window indices,
//            or null if nothing is selected.
// selDrag  — non-null while the user is shift-dragging; holds {w0}, the anchor
//            window index where the drag started.
// mousePos — last known cursor position inside the spectrogram area:
//            {col, row} in canvas pixels, {sp, co} species/code at that point.
// drag     — non-null while panning; holds {x, y} mouse-down pixel and {v}
//            the view snapshot taken at the start of the drag.
let sel = null;
let selDrag = null;
let mousePos = null;
let drag = null;

// Helper: usable width/height of the spectrogram area (excluding axes).
function vW() { return cv.width - FAW; }
function vH() { return cv.height - AH - MH; }

// ── Selection helpers ─────────────────────────────────────────────────────────

// drawSelectionRect — paints the blue highlight band for the current selection
// onto the visible canvas (cv).  Called from repaint() so it always sits on top
// of the clean ov snapshot.  Converts window indices to canvas pixel columns
// using the current view transform.
function drawSelectionRect(w, h) {
  if (!sel) return;
  const xS = view.x1 - view.x0;
  // Map the inclusive [w0,w1] window range to pixel columns, clamped to canvas.
  const sx0 = Math.round((Math.min(sel.w0, sel.w1) - view.x0) / xS * w);
  const sx1 = Math.round((Math.max(sel.w0, sel.w1) + 1 - view.x0) / xS * w);
  const lx = Math.max(0, sx0), rx = Math.min(w, sx1);
  if (rx <= lx) return;
  // Semi-transparent fill across the full spectrogram height.
  ct.fillStyle = 'rgba(80,160,255,0.15)';
  ct.fillRect(FAW + lx, MH, rx - lx, h);
  // Solid vertical boundary lines at each edge, extending into the strip above.
  ct.strokeStyle = 'rgba(100,190,255,0.8)'; ct.lineWidth = 1; ct.setLineDash([]);
  ct.beginPath(); ct.moveTo(FAW + lx, 0); ct.lineTo(FAW + lx, MH + h); ct.stroke();
  ct.beginPath(); ct.moveTo(FAW + rx, 0); ct.lineTo(FAW + rx, MH + h); ct.stroke();
}

// filterTable — shows or hides pass rows in the table based on whether they
// overlap in time with the current selection.  With no selection, all rows are
// shown.  Each <tr> carries data-t0 / data-t1 attributes (seconds) set by the
// Rust HTML writer; these are compared against the selection time range.
function filterTable() {
  const t0 = sel ? Math.min(sel.w0, sel.w1) * D.ws / D.sr : null;
  const t1 = sel ? Math.max(sel.w0, sel.w1) * D.ws / D.sr : null;
  document.querySelectorAll('tr[data-t0]').forEach(tr => {
    if (!sel) { tr.style.display = ''; return; }
    const rt0 = +tr.dataset.t0, rt1 = +tr.dataset.t1;
    // Show the row when its time range overlaps [t0,t1] (inclusive).
    tr.style.display = (rt0 <= t1 && rt1 >= t0) ? '' : 'none';
  });
}

// ── repaint ───────────────────────────────────────────────────────────────────
// Cheap redraw: restore the clean ov snapshot then add transient overlays.
// Called on every mousemove, mouseup, and selection change — never triggers
// the expensive pixel loop in render().
function repaint() {
  // Restore the clean spectrogram + axes from the offscreen snapshot.
  ct.drawImage(ov, 0, 0);
  const w = vW(), h = vH();

  // Draw the selection highlight if one exists.
  drawSelectionRect(w, h);

  // Draw the crosshair and species label while the cursor is inside the
  // spectrogram and no drag is in progress (dragging uses render() instead).
  if (mousePos && !drag && !selDrag) {
    const { col, row, sp, co } = mousePos;
    // Dashed white crosshair lines — vertical (time) and horizontal (frequency).
    ct.strokeStyle = 'rgba(255,255,255,0.5)'; ct.lineWidth = 1; ct.setLineDash([3, 3]);
    ct.beginPath(); ct.moveTo(FAW + col, 0); ct.lineTo(FAW + col, MH + h); ct.stroke();
    ct.beginPath(); ct.moveTo(FAW, MH + row); ct.lineTo(FAW + w, MH + row); ct.stroke();
    ct.setLineDash([]);
    // Species label bubble — only shown when the cursor is inside a detected pass.
    if (sp) {
      const label = co + ' \u00b7 ' + sp;  // e.g. "PIPPYG · Soprano pipistrelle"
      ct.font = '11px monospace';
      const tw = ct.measureText(label).width;
      // Position to the right of the cursor, but clamp so it stays on-canvas.
      const bx = Math.min(FAW + col + 14, FAW + w - tw - 10);
      const by = Math.max(MH + row - 10, MH + 15);
      ct.fillStyle = 'rgba(0,0,20,0.78)'; ct.fillRect(bx - 4, by - 13, tw + 8, 17);
      ct.fillStyle = '#ffdd88'; ct.textAlign = 'left'; ct.fillText(label, bx, by);
    }
  }
}

// ── render ────────────────────────────────────────────────────────────────────
// Full spectrogram redraw.  Expensive — iterates over every canvas pixel.
// Called when the view changes (zoom/pan/resize) or on first load.
// After drawing, saves the result to ov so repaint() can restore it cheaply.
function render() {
  const w = vW(), h = vH();
  ct.fillStyle = '#000'; ct.fillRect(0, 0, cv.width, cv.height);

  // ── Spectrogram pixels ──────────────────────────────────────────────────────
  // For each canvas pixel (col, row), map back to a (window, bin) index in D,
  // look up the byte value (0–255), and colourise with CMAP.
  const id = ct.createImageData(w, h); const px = id.data;
  const xS = view.x1 - view.x0, yS = view.y1 - view.y0;
  for (let col = 0; col < w; col++) {
    const wi = Math.min(D.nW - 1, Math.floor(view.x0 + col * xS / w));
    for (let row = 0; row < h; row++) {
      // Rows increase downward on canvas but frequency increases upward, so invert row.
      const bi = Math.min(D.nB - 1, Math.max(0, Math.floor(view.y0 + (h - 1 - row) * yS / h)));
      const v = D.bytes[wi * D.nB + bi];
      const ci = v * 3, off = (row * w + col) * 4;
      px[off] = CMAP[ci]; px[off + 1] = CMAP[ci + 1]; px[off + 2] = CMAP[ci + 2]; px[off + 3] = 255;
    }
  }
  ct.putImageData(id, FAW, MH);

  // ── Detection strip ─────────────────────────────────────────────────────────
  // The MH-pixel band above the spectrogram is coloured red where D.det[wi] is
  // true (bat energy detected) and near-black otherwise.
  for (let col = 0; col < w; col++) {
    const wi = Math.min(D.nW - 1, Math.floor(view.x0 + col * xS / w));
    ct.fillStyle = D.det[wi] ? '#dc3232' : '#1a1a1a';
    ct.fillRect(FAW + col, 0, 1, MH);
  }

  // ── Call group boundaries ───────────────────────────────────────────────────
  // White vertical lines at the start and end+1 window of each call group,
  // spanning the full spectrogram height including the detection strip.
  for (const c of D.calls) {
    [c.s, c.e + 1].forEach(win => {
      const x = Math.round((win - view.x0) / xS * w);
      if (x >= 0 && x <= w) {
        ct.strokeStyle = 'rgba(255,255,255,0.6)'; ct.lineWidth = 1;
        ct.beginPath(); ct.moveTo(FAW + x, 0); ct.lineTo(FAW + x, MH + h); ct.stroke();
      }
    });
  }

  // ── Band-limit reference lines ──────────────────────────────────────────────
  // Dashed white horizontal lines at 20 kHz and 120 kHz mark the bat detection
  // band boundaries.  Only drawn when within the current frequency view.
  [20000, 120000].forEach(hz => {
    const bin = hz / D.hpb;
    if (bin < view.y0 || bin > view.y1) return;
    const y = MH + h - 1 - Math.round((bin - view.y0) / yS * h);
    ct.strokeStyle = 'rgba(255,255,255,0.3)'; ct.lineWidth = 1; ct.setLineDash([4, 4]);
    ct.beginPath(); ct.moveTo(FAW, y); ct.lineTo(FAW + w, y); ct.stroke();
    ct.setLineDash([]);
  });

  // Draw axis labels, then snapshot the clean state into ov.
  drawFreqAxis(w, h); drawTimeAxis(w, h);
  oc.drawImage(cv, 0, 0); // save clean state for cheap repaint()
  repaint();              // apply selection + crosshair on top
}

// ── drawFreqAxis ──────────────────────────────────────────────────────────────
// Draws the frequency axis in the FAW-wide left gutter.
// Tick spacing adapts to the visible frequency range: wider range → coarser ticks.
// Labels are in kHz; the "Frequency" axis title is rotated 90°.
function drawFreqAxis(w, h) {
  ct.fillStyle = '#111'; ct.fillRect(0, MH, FAW, h);
  const fL = view.y0 * D.hpb, fH = view.y1 * D.hpb, fR = fH - fL;
  // Choose tick interval based on visible frequency span.
  let tk = 10000;
  if (fR < 5000) tk = 500; else if (fR < 12000) tk = 2000; else if (fR < 30000) tk = 5000;
  const first = Math.ceil(fL / tk) * tk;
  ct.font = '10px monospace'; ct.textAlign = 'right'; ct.fillStyle = '#777';
  for (let f = first; f <= fH; f += tk) {
    const bin = f / D.hpb;
    const y = MH + h - 1 - Math.round((bin - view.y0) / (view.y1 - view.y0) * h);
    if (y < MH || y > MH + h) continue;
    // Label: "45k" for exact multiples of 1 kHz, "22.5k" otherwise.
    ct.fillText((f / 1000).toFixed(f % 1000 ? 1 : 0) + 'k', FAW - 3, y + 3);
    ct.strokeStyle = '#333'; ct.lineWidth = 1;
    ct.beginPath(); ct.moveTo(FAW - 5, y); ct.lineTo(FAW, y); ct.stroke();
  }
  // Rotated "Frequency" label centred on the axis gutter.
  ct.save(); ct.translate(8, MH + h / 2); ct.rotate(-Math.PI / 2);
  ct.fillStyle = '#555'; ct.textAlign = 'center'; ct.font = '10px monospace';
  ct.fillText('Frequency', 0, 0); ct.restore();
}

// ── drawTimeAxis ──────────────────────────────────────────────────────────────
// Draws the time axis in the AH-pixel strip below the spectrogram.
// Tick spacing adapts from 0.05 s (very zoomed in) to 30 s (full recording).
// Labels switch from "1.23s" to "60s" format at 5-second visible span.
function drawTimeAxis(w, h) {
  const y0 = MH + h;
  ct.fillStyle = '#111'; ct.fillRect(FAW, y0, w, AH);
  // Convert window indices to seconds for the visible range.
  const t0 = view.x0 * D.ws / D.sr, t1 = view.x1 * D.ws / D.sr, tS = t1 - t0;
  // Choose tick interval based on visible time span.
  let tk = 1;
  if (tS > 120) tk = 30; else if (tS > 60) tk = 10; else if (tS > 20) tk = 5; else if (tS > 10) tk = 2;
  else if (tS < 0.5) tk = 0.05; else if (tS < 1) tk = 0.1; else if (tS < 3) tk = 0.5;
  const first = Math.ceil(t0 / tk) * tk;
  ct.font = '10px monospace'; ct.textAlign = 'center'; ct.fillStyle = '#777';
  ct.strokeStyle = '#333'; ct.lineWidth = 1;
  // Round each tick to avoid floating-point drift accumulating across many ticks.
  for (let t = first; t <= t1 + 1e-9; t = Math.round((t + tk) * 10000) / 10000) {
    const win = t * D.sr / D.ws;
    const x = Math.round((win - view.x0) / (view.x1 - view.x0) * w);
    if (x < 0 || x > w) continue;
    ct.beginPath(); ct.moveTo(FAW + x, y0); ct.lineTo(FAW + x, y0 + 5); ct.stroke();
    ct.fillText(tS < 5 ? t.toFixed(2) + 's' : t.toFixed(0) + 's', FAW + x, y0 + 16);
  }
}

// ── resize ────────────────────────────────────────────────────────────────────
// Resizes both canvases to fill the browser window, then triggers a full render.
// Called on window resize and once on initial load.
function resize() {
  cv.width = Math.max(800, document.documentElement.clientWidth - 2);
  cv.height = Math.max(400, Math.floor(window.innerHeight * 0.62));
  ov.width = cv.width; ov.height = cv.height;
  render();
}

// ── Scroll wheel — zoom ───────────────────────────────────────────────────────
// Plain scroll  → zoom time axis (x) around the cursor position.
// Shift + scroll → zoom frequency axis (y) around the cursor position.
// Zoom factor: 1.25× out (deltaY > 0) or 0.8× in, keeping the point under
// the cursor stationary by adjusting the view centre.
cv.addEventListener('wheel', function(e) {
  e.preventDefault();
  const r = cv.getBoundingClientRect();
  const cx = e.clientX - r.left - FAW, cy = e.clientY - r.top - MH;
  const w = vW(), h = vH();
  const zf = e.deltaY > 0 ? 1.25 : 0.8;
  if (e.shiftKey) {
    // Zoom frequency: keep the bin under the cursor fixed.
    const fy = 1 - Math.max(0, Math.min(1, cy / h));
    const cen = view.y0 + fy * (view.y1 - view.y0); const span = (view.y1 - view.y0) * zf;
    view.y0 = Math.max(0, cen - fy * span); view.y1 = Math.min(D.nB, view.y0 + span);
  } else {
    // Zoom time: keep the window under the cursor fixed.
    const fx = Math.max(0, Math.min(1, cx / w));
    const cen = view.x0 + fx * (view.x1 - view.x0); const span = (view.x1 - view.x0) * zf;
    view.x0 = Math.max(0, cen - fx * span); view.x1 = Math.min(D.nW, view.x0 + span);
  }
  render();
}, { passive: false });

// ── Mouse down — start pan or selection ──────────────────────────────────────
// Shift + mousedown → begin a time-range selection drag (selDrag).
// Plain mousedown   → begin a pan drag.
cv.addEventListener('mousedown', function(e) {
  const r = cv.getBoundingClientRect();
  const col = e.clientX - r.left - FAW, w = vW();
  if (e.shiftKey) {
    // Anchor the selection at the window index under the cursor.
    const wi = Math.max(0, Math.min(D.nW - 1, Math.floor(view.x0 + col / w * (view.x1 - view.x0))));
    selDrag = { w0: wi }; sel = { w0: wi, w1: wi };
    repaint();
  } else {
    // Snapshot the current view and cursor position for pan delta calculation.
    drag = { x: e.clientX, y: e.clientY, v: { ...view } };
  }
  e.preventDefault();
});

// ── Mouse move — pan, selection drag, crosshair ───────────────────────────────
// Handles three concurrent concerns:
//   1. If panning (drag set): update view from delta and call render().
//   2. If shift-dragging (selDrag set): extend sel.w1 and filter the table.
//   3. Always: update the info bar and mousePos for the crosshair, then repaint().
window.addEventListener('mousemove', function(e) {
  if (drag) {
    const w = vW(), h = vH();
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    const xS = drag.v.x1 - drag.v.x0, yS = drag.v.y1 - drag.v.y0;
    // Translate view by the pixel delta, clamped to data bounds.
    let x0 = drag.v.x0 - dx / w * xS; x0 = Math.max(0, Math.min(D.nW - xS, x0));
    let y0 = drag.v.y0 + dy / h * yS; y0 = Math.max(0, Math.min(D.nB - yS, y0));
    view = { x0, x1: x0 + xS, y0, y1: y0 + yS }; render();
  }
  const r = cv.getBoundingClientRect();
  const col = e.clientX - r.left - FAW, row = e.clientY - r.top - MH;
  const w = vW(), h = vH();
  if (col >= 0 && col < w && row >= 0 && row < h) {
    // Cursor is inside the spectrogram area — compute data coordinates.
    const wi = Math.min(D.nW - 1, Math.max(0, Math.floor(view.x0 + col / w * (view.x1 - view.x0))));
    const bi = Math.min(D.nB - 1, Math.max(0, Math.floor(view.y0 + (h - 1 - row) / h * (view.y1 - view.y0))));
    if (selDrag) {
      // Extend the selection to the current window index.
      sel = { w0: selDrag.w0, w1: wi };
      filterTable();
    }
    const tsec = wi * D.ws / D.sr;
    const fkhz = bi * D.hpb / 1000;
    // Convert the stored byte value back to dB: byte = (db+80)/80*255, so db = byte/255*80−80.
    const bv = D.bytes[wi * D.nB + bi];
    const db = bv > 0 ? (bv / 255 * 80 - 80).toFixed(1) : '\u221280';
    // Look up which pass (if any) the current time falls inside.
    let sp = '', co = '';
    for (const p of D.passes) { if (tsec >= p.t0 && tsec <= p.t1) { sp = p.sp; co = p.co; break; } }
    // Update the status bar below the canvas.
    document.getElementById('info').textContent =
      't = ' + tsec.toFixed(3) + ' s\u2003|\u2003f = ' + fkhz.toFixed(2) + ' kHz\u2003|\u2003' + db + ' dB' + (sp ? '\u2003|\u2003' + co + ' \u00b7 ' + sp : '');
    mousePos = { col, row, sp, co };
    repaint();
  } else {
    // Cursor has left the spectrogram area — clear crosshair.
    mousePos = null; repaint();
    document.getElementById('info').innerHTML = '&nbsp;';
  }
});

// ── Mouse up — end pan or selection drag ──────────────────────────────────────
window.addEventListener('mouseup', function() {
  if (selDrag) { selDrag = null; filterTable(); repaint(); }
  drag = null;
});

// ── Mouse leave — clear crosshair ─────────────────────────────────────────────
cv.addEventListener('mouseleave', function() { mousePos = null; repaint(); });

// ── Double-click — reset view ─────────────────────────────────────────────────
// Returns to the full recording view and clears any active selection.
cv.addEventListener('dblclick', function() {
  view = { x0: 0, x1: D.nW, y0: 0, y1: D.nB };
  sel = null; filterTable(); render();
});

// ── Keyboard — Escape clears selection ────────────────────────────────────────
document.addEventListener('keydown', function(e) {
  if (e.key === 'Escape') { sel = null; selDrag = null; filterTable(); repaint(); }
});

// ── Table row clicks — zoom to pass ───────────────────────────────────────────
// Each pass row in the table has data-t0 / data-t1 attributes (seconds).
// Clicking a row zooms the spectrogram to that time range (with a small pad)
// and scrolls the canvas into view.
document.querySelectorAll('tr[data-t0]').forEach(tr => {
  tr.addEventListener('click', function() {
    const t0 = +this.dataset.t0, t1 = +this.dataset.t1;
    const s = t0 * D.sr / D.ws, e = t1 * D.sr / D.ws;
    // Pad by 10% of the pass duration or at least 5 windows.
    const pad = Math.max(5, (e - s) * 0.1) | 0;
    view.x0 = Math.max(0, s - pad); view.x1 = Math.min(D.nW, e + pad);
    render(); cv.scrollIntoView({ behavior: 'smooth' });
  });
});

// ── Initial render ────────────────────────────────────────────────────────────
window.addEventListener('resize', resize);
resize(); // sets canvas dimensions and triggers first render
