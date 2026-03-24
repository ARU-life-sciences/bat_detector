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
const cv = document.getElementById('cv'); const ct = cv.getContext('2d');
// ov: clean render (spectrogram + axes + boundaries, no selection, no crosshair).
// repaint() composites ov + selection + crosshair onto cv cheaply.
const ov = document.createElement('canvas'); const oc = ov.getContext('2d');
const MH = 10, AH = 24, FAW = 56;
let view = { x0: 0, x1: D.nW, y0: 0, y1: D.nB };
let sel = null;      // {w0,w1} selected window-index range, or null
let selDrag = null;  // {w0} anchor window while shift-dragging
let mousePos = null; // {col,row,sp,co} last mouse position inside spectrogram
let drag = null;
function vW() { return cv.width - FAW; }
function vH() { return cv.height - AH - MH; }

// ── Selection helpers ─────────────────────────────────────────────────────────
function drawSelectionRect(w, h) {
  if (!sel) return;
  const xS = view.x1 - view.x0;
  const sx0 = Math.round((Math.min(sel.w0, sel.w1) - view.x0) / xS * w);
  const sx1 = Math.round((Math.max(sel.w0, sel.w1) + 1 - view.x0) / xS * w);
  const lx = Math.max(0, sx0), rx = Math.min(w, sx1);
  if (rx <= lx) return;
  ct.fillStyle = 'rgba(80,160,255,0.15)';
  ct.fillRect(FAW + lx, MH, rx - lx, h);
  ct.strokeStyle = 'rgba(100,190,255,0.8)'; ct.lineWidth = 1; ct.setLineDash([]);
  ct.beginPath(); ct.moveTo(FAW + lx, 0); ct.lineTo(FAW + lx, MH + h); ct.stroke();
  ct.beginPath(); ct.moveTo(FAW + rx, 0); ct.lineTo(FAW + rx, MH + h); ct.stroke();
}
function filterTable() {
  const t0 = sel ? Math.min(sel.w0, sel.w1) * D.ws / D.sr : null;
  const t1 = sel ? Math.max(sel.w0, sel.w1) * D.ws / D.sr : null;
  document.querySelectorAll('tr[data-t0]').forEach(tr => {
    if (!sel) { tr.style.display = ''; return; }
    const rt0 = +tr.dataset.t0, rt1 = +tr.dataset.t1;
    tr.style.display = (rt0 <= t1 && rt1 >= t0) ? '' : 'none';
  });
}

// ── repaint: cheap composite of ov + selection + crosshair ───────────────────
function repaint() {
  ct.drawImage(ov, 0, 0);
  const w = vW(), h = vH();
  drawSelectionRect(w, h);
  if (mousePos && !drag && !selDrag) {
    const { col, row, sp, co } = mousePos;
    ct.strokeStyle = 'rgba(255,255,255,0.5)'; ct.lineWidth = 1; ct.setLineDash([3, 3]);
    ct.beginPath(); ct.moveTo(FAW + col, 0); ct.lineTo(FAW + col, MH + h); ct.stroke();
    ct.beginPath(); ct.moveTo(FAW, MH + row); ct.lineTo(FAW + w, MH + row); ct.stroke();
    ct.setLineDash([]);
    if (sp) {
      const label = co + ' \u00b7 ' + sp;
      ct.font = '11px monospace';
      const tw = ct.measureText(label).width;
      const bx = Math.min(FAW + col + 14, FAW + w - tw - 10);
      const by = Math.max(MH + row - 10, MH + 15);
      ct.fillStyle = 'rgba(0,0,20,0.78)'; ct.fillRect(bx - 4, by - 13, tw + 8, 17);
      ct.fillStyle = '#ffdd88'; ct.textAlign = 'left'; ct.fillText(label, bx, by);
    }
  }
}

// ── render: full spectrogram redraw → save to ov → repaint ───────────────────
function render() {
  const w = vW(), h = vH();
  ct.fillStyle = '#000'; ct.fillRect(0, 0, cv.width, cv.height);
  const id = ct.createImageData(w, h); const px = id.data;
  const xS = view.x1 - view.x0, yS = view.y1 - view.y0;
  for (let col = 0; col < w; col++) {
    const wi = Math.min(D.nW - 1, Math.floor(view.x0 + col * xS / w));
    for (let row = 0; row < h; row++) {
      const bi = Math.min(D.nB - 1, Math.max(0, Math.floor(view.y0 + (h - 1 - row) * yS / h)));
      const v = D.bytes[wi * D.nB + bi];
      const ci = v * 3, off = (row * w + col) * 4;
      px[off] = CMAP[ci]; px[off + 1] = CMAP[ci + 1]; px[off + 2] = CMAP[ci + 2]; px[off + 3] = 255;
    }
  }
  ct.putImageData(id, FAW, MH);
  for (let col = 0; col < w; col++) {
    const wi = Math.min(D.nW - 1, Math.floor(view.x0 + col * xS / w));
    ct.fillStyle = D.det[wi] ? '#dc3232' : '#1a1a1a';
    ct.fillRect(FAW + col, 0, 1, MH);
  }
  for (const c of D.calls) {
    [c.s, c.e + 1].forEach(win => {
      const x = Math.round((win - view.x0) / xS * w);
      if (x >= 0 && x <= w) {
        ct.strokeStyle = 'rgba(255,255,255,0.6)'; ct.lineWidth = 1;
        ct.beginPath(); ct.moveTo(FAW + x, 0); ct.lineTo(FAW + x, MH + h); ct.stroke();
      }
    });
  }
  [20000, 120000].forEach(hz => {
    const bin = hz / D.hpb;
    if (bin < view.y0 || bin > view.y1) return;
    const y = MH + h - 1 - Math.round((bin - view.y0) / yS * h);
    ct.strokeStyle = 'rgba(255,255,255,0.3)'; ct.lineWidth = 1; ct.setLineDash([4, 4]);
    ct.beginPath(); ct.moveTo(FAW, y); ct.lineTo(FAW + w, y); ct.stroke();
    ct.setLineDash([]);
  });
  drawFreqAxis(w, h); drawTimeAxis(w, h);
  oc.drawImage(cv, 0, 0); // save clean state
  repaint();            // apply selection + crosshair on top
}
function drawFreqAxis(w, h) {
  ct.fillStyle = '#111'; ct.fillRect(0, MH, FAW, h);
  const fL = view.y0 * D.hpb, fH = view.y1 * D.hpb, fR = fH - fL;
  let tk = 10000;
  if (fR < 5000) tk = 500; else if (fR < 12000) tk = 2000; else if (fR < 30000) tk = 5000;
  const first = Math.ceil(fL / tk) * tk;
  ct.font = '10px monospace'; ct.textAlign = 'right'; ct.fillStyle = '#777';
  for (let f = first; f <= fH; f += tk) {
    const bin = f / D.hpb;
    const y = MH + h - 1 - Math.round((bin - view.y0) / (view.y1 - view.y0) * h);
    if (y < MH || y > MH + h) continue;
    ct.fillText((f / 1000).toFixed(f % 1000 ? 1 : 0) + 'k', FAW - 3, y + 3);
    ct.strokeStyle = '#333'; ct.lineWidth = 1;
    ct.beginPath(); ct.moveTo(FAW - 5, y); ct.lineTo(FAW, y); ct.stroke();
  }
  ct.save(); ct.translate(8, MH + h / 2); ct.rotate(-Math.PI / 2);
  ct.fillStyle = '#555'; ct.textAlign = 'center'; ct.font = '10px monospace';
  ct.fillText('Frequency', 0, 0); ct.restore();
}
function drawTimeAxis(w, h) {
  const y0 = MH + h;
  ct.fillStyle = '#111'; ct.fillRect(FAW, y0, w, AH);
  const t0 = view.x0 * D.ws / D.sr, t1 = view.x1 * D.ws / D.sr, tS = t1 - t0;
  let tk = 1;
  if (tS > 120) tk = 30; else if (tS > 60) tk = 10; else if (tS > 20) tk = 5; else if (tS > 10) tk = 2;
  else if (tS < 0.5) tk = 0.05; else if (tS < 1) tk = 0.1; else if (tS < 3) tk = 0.5;
  const first = Math.ceil(t0 / tk) * tk;
  ct.font = '10px monospace'; ct.textAlign = 'center'; ct.fillStyle = '#777';
  ct.strokeStyle = '#333'; ct.lineWidth = 1;
  for (let t = first; t <= t1 + 1e-9; t = Math.round((t + tk) * 10000) / 10000) {
    const win = t * D.sr / D.ws;
    const x = Math.round((win - view.x0) / (view.x1 - view.x0) * w);
    if (x < 0 || x > w) continue;
    ct.beginPath(); ct.moveTo(FAW + x, y0); ct.lineTo(FAW + x, y0 + 5); ct.stroke();
    ct.fillText(tS < 5 ? t.toFixed(2) + 's' : t.toFixed(0) + 's', FAW + x, y0 + 16);
  }
}
function resize() {
  cv.width = Math.max(800, document.documentElement.clientWidth - 2);
  cv.height = Math.max(400, Math.floor(window.innerHeight * 0.62));
  ov.width = cv.width; ov.height = cv.height;
  render();
}
cv.addEventListener('wheel', function(e) {
  e.preventDefault();
  const r = cv.getBoundingClientRect();
  const cx = e.clientX - r.left - FAW, cy = e.clientY - r.top - MH;
  const w = vW(), h = vH();
  const zf = e.deltaY > 0 ? 1.25 : 0.8;
  if (e.shiftKey) {
    const fy = 1 - Math.max(0, Math.min(1, cy / h));
    const cen = view.y0 + fy * (view.y1 - view.y0); const span = (view.y1 - view.y0) * zf;
    view.y0 = Math.max(0, cen - fy * span); view.y1 = Math.min(D.nB, view.y0 + span);
  } else {
    const fx = Math.max(0, Math.min(1, cx / w));
    const cen = view.x0 + fx * (view.x1 - view.x0); const span = (view.x1 - view.x0) * zf;
    view.x0 = Math.max(0, cen - fx * span); view.x1 = Math.min(D.nW, view.x0 + span);
  }
  render();
}, { passive: false });
cv.addEventListener('mousedown', function(e) {
  const r = cv.getBoundingClientRect();
  const col = e.clientX - r.left - FAW, w = vW();
  if (e.shiftKey) {
    // Start time-range selection
    const wi = Math.max(0, Math.min(D.nW - 1, Math.floor(view.x0 + col / w * (view.x1 - view.x0))));
    selDrag = { w0: wi }; sel = { w0: wi, w1: wi };
    repaint();
  } else {
    drag = { x: e.clientX, y: e.clientY, v: { ...view } };
  }
  e.preventDefault();
});
window.addEventListener('mousemove', function(e) {
  if (drag) {
    const w = vW(), h = vH();
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    const xS = drag.v.x1 - drag.v.x0, yS = drag.v.y1 - drag.v.y0;
    let x0 = drag.v.x0 - dx / w * xS; x0 = Math.max(0, Math.min(D.nW - xS, x0));
    let y0 = drag.v.y0 + dy / h * yS; y0 = Math.max(0, Math.min(D.nB - yS, y0));
    view = { x0, x1: x0 + xS, y0, y1: y0 + yS }; render();
  }
  const r = cv.getBoundingClientRect();
  const col = e.clientX - r.left - FAW, row = e.clientY - r.top - MH;
  const w = vW(), h = vH();
  if (col >= 0 && col < w && row >= 0 && row < h) {
    const wi = Math.min(D.nW - 1, Math.max(0, Math.floor(view.x0 + col / w * (view.x1 - view.x0))));
    const bi = Math.min(D.nB - 1, Math.max(0, Math.floor(view.y0 + (h - 1 - row) / h * (view.y1 - view.y0))));
    if (selDrag) {
      sel = { w0: selDrag.w0, w1: wi };
      filterTable();
    }
    const tsec = wi * D.ws / D.sr;
    const fkhz = bi * D.hpb / 1000;
    const bv = D.bytes[wi * D.nB + bi];
    const db = bv > 0 ? (bv / 255 * 80 - 80).toFixed(1) : '\u221280';
    let sp = '', co = '';
    for (const p of D.passes) { if (tsec >= p.t0 && tsec <= p.t1) { sp = p.sp; co = p.co; break; } }
    document.getElementById('info').textContent =
      't = ' + tsec.toFixed(3) + ' s\u2003|\u2003f = ' + fkhz.toFixed(2) + ' kHz\u2003|\u2003' + db + ' dB' + (sp ? '\u2003|\u2003' + co + ' \u00b7 ' + sp : '');
    mousePos = { col, row, sp, co };
    repaint();
  } else {
    mousePos = null; repaint();
    document.getElementById('info').innerHTML = '&nbsp;';
  }
});
window.addEventListener('mouseup', function() {
  if (selDrag) { selDrag = null; filterTable(); repaint(); }
  drag = null;
});
cv.addEventListener('mouseleave', function() { mousePos = null; repaint(); });
cv.addEventListener('dblclick', function() {
  view = { x0: 0, x1: D.nW, y0: 0, y1: D.nB };
  sel = null; filterTable(); render();
});
document.addEventListener('keydown', function(e) {
  if (e.key === 'Escape') { sel = null; selDrag = null; filterTable(); repaint(); }
});
document.querySelectorAll('tr[data-t0]').forEach(tr => {
  tr.addEventListener('click', function() {
    const t0 = +this.dataset.t0, t1 = +this.dataset.t1;
    const s = t0 * D.sr / D.ws, e = t1 * D.sr / D.ws;
    const pad = Math.max(5, (e - s) * 0.1) | 0;
    view.x0 = Math.max(0, s - pad); view.x1 = Math.min(D.nW, e + pad);
    render(); cv.scrollIntoView({ behavior: 'smooth' });
  });
});
window.addEventListener('resize', resize);
resize();
