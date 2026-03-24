use std::io::Write as _;

use image::{ImageBuffer, Rgb};

use crate::features::CallFeatures;

// ── Colour map ────────────────────────────────────────────────────────────────

/// Maps t ∈ [0, 1] → heat colour: black → blue → cyan → green → yellow → red.
pub fn colormap(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    if t < 0.25 {
        let s = t * 4.0;
        [0, 0, (255.0 * s) as u8]
    } else if t < 0.5 {
        let s = (t - 0.25) * 4.0;
        [0, (255.0 * s) as u8, 255]
    } else if t < 0.75 {
        let s = (t - 0.5) * 4.0;
        [(255.0 * s) as u8, 255, (255.0 * (1.0 - s)) as u8]
    } else {
        let s = (t - 0.75) * 4.0;
        [255, (255.0 * (1.0 - s)) as u8, 0]
    }
}

// ── Output data structures ────────────────────────────────────────────────────

pub struct PeakInfo {
    pub features: CallFeatures,
    pub species: &'static str,
    pub notes: &'static str,
}

/// One call group, potentially containing multiple simultaneous species.
pub struct CallGroupInfo {
    pub start_win: usize,
    pub end_win: usize,
    pub start_sec: f32,
    pub end_sec: f32,
    pub duration_ms: f32,
    pub peaks: Vec<PeakInfo>,
}

// ── Species-pass aggregation ──────────────────────────────────────────────────

/// One "pass" = all consecutive call groups of the same species within
/// `max_gap_sec` of each other.  When two species overlap in time they each
/// get their own PassInfo, giving one table row per species per pass.
pub struct PassInfo {
    pub species: &'static str,
    pub start_sec: f32,
    pub end_sec: f32,
    pub n_pulses: usize,
    /// Additional sub-threshold pulses found by the local search (single-pulse passes only).
    pub n_extra: usize,
    pub mean_peak_hz: f32,
    /// True when this single-pulse pass is entirely nested within a larger pass of a
    /// different species, making the identification unreliable.
    pub dubious: bool,
}

/// Group call groups into species passes.  Calls of the same species separated
/// by ≤ `max_gap_sec` are merged; different species are always kept separate.
pub fn compute_passes(calls: &[CallGroupInfo], max_gap_sec: f32) -> Vec<PassInfo> {
    let mut by_species: std::collections::HashMap<&'static str, Vec<(f32, f32, f32)>> =
        std::collections::HashMap::new();

    for call in calls {
        for peak in &call.peaks {
            by_species
                .entry(peak.species)
                .or_default()
                .push((call.start_sec, call.end_sec, peak.features.peak_hz));
        }
    }

    let mut passes: Vec<PassInfo> = Vec::new();

    for (species, mut items) in by_species {
        items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (mut cur_start, mut cur_end, first_hz) = items[0];
        let mut hz_sum = first_hz;
        let mut count = 1usize;

        for &(start, end, peak_hz) in &items[1..] {
            if start - cur_end <= max_gap_sec {
                if end > cur_end { cur_end = end; }
                hz_sum += peak_hz;
                count += 1;
            } else {
                passes.push(PassInfo {
                    species,
                    start_sec: cur_start,
                    end_sec: cur_end,
                    n_pulses: count,
                    n_extra: 0,
                    mean_peak_hz: hz_sum / count as f32,
                    dubious: false,
                });
                cur_start = start;
                cur_end = end;
                hz_sum = peak_hz;
                count = 1;
            }
        }
        passes.push(PassInfo {
            species,
            start_sec: cur_start,
            end_sec: cur_end,
            n_pulses: count,
            n_extra: 0,
            mean_peak_hz: hz_sum / count as f32,
            dubious: false,
        });
    }

    passes.sort_by(|a, b| a.start_sec.partial_cmp(&b.start_sec).unwrap());
    passes
}

// ── PNG output ────────────────────────────────────────────────────────────────

/// Write a spectrogram PNG.
///
/// `spec_bytes` is window-major (`bytes[w * freq_bins + b]`), with values
/// already normalised to 0–255.
pub fn write_png(
    stem: &str,
    spec_bytes: &[u8],
    detected: &[bool],
    n_windows: usize,
    freq_bins: usize,
    bin_low: usize,
    bin_high: usize,
) -> Result<(), image::ImageError> {
    const MARKER_H: u32 = 8;
    let w = n_windows as u32;
    let h = freq_bins as u32 + MARKER_H;
    let mut img = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(w, h);

    for (x, win_bytes) in spec_bytes.chunks(freq_bins).enumerate().take(n_windows) {
        // Detection marker strip
        let mc = if detected[x] { Rgb([220u8, 50, 50]) } else { Rgb([30u8, 30, 30]) };
        for y in 0..MARKER_H {
            img.put_pixel(x as u32, y, mc);
        }
        // Spectrogram pixels
        for (bin, &byte) in win_bytes.iter().enumerate() {
            let [r, g, b] = colormap(byte as f32 / 255.0);
            let y = MARKER_H + (freq_bins as u32 - 1 - bin as u32);
            img.put_pixel(x as u32, y, Rgb([r, g, b]));
        }
    }

    // Vertical white lines at call group boundaries
    for x in 1..w {
        if detected[x as usize] != detected[x as usize - 1] {
            for y in 0..h {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
    }

    // Horizontal lines at bat-band boundaries
    let y_low = MARKER_H + (freq_bins as u32 - 1 - bin_low as u32);
    let y_high = MARKER_H + (freq_bins as u32 - 1 - bin_high as u32);
    for x in 0..w {
        img.put_pixel(x, y_low, Rgb([255, 255, 255]));
        img.put_pixel(x, y_high, Rgb([255, 255, 255]));
    }

    let path = format!("{}_spectrogram.png", stem);
    img.save(&path)?;
    println!("Spectrogram PNG saved to:   {}", path);
    Ok(())
}

// ── HTML output ───────────────────────────────────────────────────────────────

// Static CSS — written verbatim; no Rust format! substitution.
const HTML_CSS: &str = r#"<style>
*{box-sizing:border-box;margin:0;padding:0}
body{background:#0d0d1a;color:#c8c8dc;font:13px/1.5 'Courier New',monospace;overflow-x:hidden}
h1{padding:12px 16px 2px;font-size:15px;font-weight:normal;color:#88aaff}
.meta{padding:2px 16px 6px;color:#666;font-size:11px}
.help{padding:0 16px 8px;color:#444;font-size:11px}
#wrap{background:#000;line-height:0}
canvas{display:block;cursor:crosshair;user-select:none;-webkit-user-select:none}
#info{padding:4px 16px;color:#aaccff;font-size:12px;min-height:20px}
#calls{padding:8px 16px 20px}
#calls h2{font-size:13px;font-weight:normal;color:#88aaff;margin-bottom:8px}
table{border-collapse:collapse;font-size:12px;width:100%}
th,td{padding:3px 14px 3px 0;text-align:left;vertical-align:top}
th{color:#555;font-weight:normal;border-bottom:1px solid #222;padding-bottom:5px}
.group-first td{border-top:1px solid #2a2a40}
tr[data-t0]{cursor:pointer}
tr[data-t0]:hover td{background:#1a1a3a;color:#eee}
tr.dubious{opacity:0.4}
tr.dubious:hover{opacity:1}
.cf{display:inline-block;padding:1px 5px;border-radius:2px;font-size:10px;background:#1a4;color:#afa}
.fm{display:inline-block;padding:1px 5px;border-radius:2px;font-size:10px;background:#148;color:#adf}
</style>
"#;

// Static JS — written verbatim after the dynamic D object and base64 data.
const HTML_JS: &str = r#"
const CMAP=(function(){
  const t=new Uint8ClampedArray(256*3);
  for(let i=0;i<256;i++){
    const v=i/255;let r,g,b;
    if(v<.25){const s=v*4;r=0;g=0;b=255*s|0;}
    else if(v<.5){const s=(v-.25)*4;r=0;g=255*s|0;b=255;}
    else if(v<.75){const s=(v-.5)*4;r=255*s|0;g=255;b=255*(1-s)|0;}
    else{const s=(v-.75)*4;r=255;g=255*(1-s)|0;b=0;}
    t[i*3]=r;t[i*3+1]=g;t[i*3+2]=b;
  }
  return t;
})();
const cv=document.getElementById('cv');
const ct=cv.getContext('2d');
const MH=10,AH=24,FAW=56;
let view={x0:0,x1:D.nW,y0:0,y1:D.nB};
function vW(){return cv.width-FAW;}
function vH(){return cv.height-AH-MH;}
function render(){
  const w=vW(),h=vH();
  ct.fillStyle='#000';ct.fillRect(0,0,cv.width,cv.height);
  const id=ct.createImageData(w,h);const px=id.data;
  const xS=view.x1-view.x0,yS=view.y1-view.y0;
  for(let col=0;col<w;col++){
    const wi=Math.min(D.nW-1,Math.floor(view.x0+col*xS/w));
    for(let row=0;row<h;row++){
      const bi=Math.min(D.nB-1,Math.max(0,Math.floor(view.y0+(h-1-row)*yS/h)));
      const v=D.bytes[wi*D.nB+bi];
      const ci=v*3,off=(row*w+col)*4;
      px[off]=CMAP[ci];px[off+1]=CMAP[ci+1];px[off+2]=CMAP[ci+2];px[off+3]=255;
    }
  }
  ct.putImageData(id,FAW,MH);
  // Detection marker strip
  for(let col=0;col<w;col++){
    const wi=Math.min(D.nW-1,Math.floor(view.x0+col*xS/w));
    ct.fillStyle=D.det[wi]?'#dc3232':'#1a1a1a';
    ct.fillRect(FAW+col,0,1,MH);
  }
  // Call group boundary lines
  for(const c of D.calls){
    [c.s,c.e+1].forEach(win=>{
      const x=Math.round((win-view.x0)/xS*w);
      if(x>=0&&x<=w){
        ct.strokeStyle='rgba(255,255,255,0.6)';ct.lineWidth=1;
        ct.beginPath();ct.moveTo(FAW+x,0);ct.lineTo(FAW+x,MH+h);ct.stroke();
      }
    });
  }
  // Dashed bat-band boundary lines at 20 kHz and 120 kHz
  [20000,120000].forEach(hz=>{
    const bin=hz/D.hpb;
    if(bin<view.y0||bin>view.y1)return;
    const y=MH+h-1-Math.round((bin-view.y0)/yS*h);
    ct.strokeStyle='rgba(255,255,255,0.3)';ct.lineWidth=1;ct.setLineDash([4,4]);
    ct.beginPath();ct.moveTo(FAW,y);ct.lineTo(FAW+w,y);ct.stroke();
    ct.setLineDash([]);
  });
  drawFreqAxis(w,h);drawTimeAxis(w,h);
}
function drawFreqAxis(w,h){
  ct.fillStyle='#111';ct.fillRect(0,MH,FAW,h);
  const fL=view.y0*D.hpb,fH=view.y1*D.hpb,fR=fH-fL;
  let tk=10000;
  if(fR<5000)tk=500;else if(fR<12000)tk=2000;else if(fR<30000)tk=5000;
  const first=Math.ceil(fL/tk)*tk;
  ct.font='10px monospace';ct.textAlign='right';ct.fillStyle='#777';
  for(let f=first;f<=fH;f+=tk){
    const bin=f/D.hpb;
    const y=MH+h-1-Math.round((bin-view.y0)/(view.y1-view.y0)*h);
    if(y<MH||y>MH+h)continue;
    ct.fillText((f/1000).toFixed(f%1000?1:0)+'k',FAW-3,y+3);
    ct.strokeStyle='#333';ct.lineWidth=1;
    ct.beginPath();ct.moveTo(FAW-5,y);ct.lineTo(FAW,y);ct.stroke();
  }
  ct.save();ct.translate(8,MH+h/2);ct.rotate(-Math.PI/2);
  ct.fillStyle='#555';ct.textAlign='center';ct.font='10px monospace';
  ct.fillText('Frequency',0,0);ct.restore();
}
function drawTimeAxis(w,h){
  const y0=MH+h;
  ct.fillStyle='#111';ct.fillRect(FAW,y0,w,AH);
  const t0=view.x0*D.ws/D.sr,t1=view.x1*D.ws/D.sr,tS=t1-t0;
  let tk=1;
  if(tS>120)tk=30;else if(tS>60)tk=10;else if(tS>20)tk=5;else if(tS>10)tk=2;
  else if(tS<0.5)tk=0.05;else if(tS<1)tk=0.1;else if(tS<3)tk=0.5;
  const first=Math.ceil(t0/tk)*tk;
  ct.font='10px monospace';ct.textAlign='center';ct.fillStyle='#777';
  ct.strokeStyle='#333';ct.lineWidth=1;
  for(let t=first;t<=t1+1e-9;t=Math.round((t+tk)*10000)/10000){
    const win=t*D.sr/D.ws;
    const x=Math.round((win-view.x0)/(view.x1-view.x0)*w);
    if(x<0||x>w)continue;
    ct.beginPath();ct.moveTo(FAW+x,y0);ct.lineTo(FAW+x,y0+5);ct.stroke();
    ct.fillText(tS<5?t.toFixed(2)+'s':t.toFixed(0)+'s',FAW+x,y0+16);
  }
}
function resize(){
  cv.width=Math.max(800,document.documentElement.clientWidth-2);
  cv.height=Math.max(400,Math.floor(window.innerHeight*0.62));
  render();
}
cv.addEventListener('wheel',function(e){
  e.preventDefault();
  const r=cv.getBoundingClientRect();
  const cx=e.clientX-r.left-FAW,cy=e.clientY-r.top-MH;
  const w=vW(),h=vH();
  const zf=e.deltaY>0?1.25:0.8;
  if(e.shiftKey){
    const fy=1-Math.max(0,Math.min(1,cy/h));
    const cen=view.y0+fy*(view.y1-view.y0);const span=(view.y1-view.y0)*zf;
    view.y0=Math.max(0,cen-fy*span);view.y1=Math.min(D.nB,view.y0+span);
  }else{
    const fx=Math.max(0,Math.min(1,cx/w));
    const cen=view.x0+fx*(view.x1-view.x0);const span=(view.x1-view.x0)*zf;
    view.x0=Math.max(0,cen-fx*span);view.x1=Math.min(D.nW,view.x0+span);
  }
  render();
},{passive:false});
let drag=null;
cv.addEventListener('mousedown',e=>{drag={x:e.clientX,y:e.clientY,v:{...view}};e.preventDefault();});
window.addEventListener('mousemove',function(e){
  if(drag){
    const w=vW(),h=vH();
    const dx=e.clientX-drag.x,dy=e.clientY-drag.y;
    const xS=drag.v.x1-drag.v.x0,yS=drag.v.y1-drag.v.y0;
    let x0=drag.v.x0-dx/w*xS;x0=Math.max(0,Math.min(D.nW-xS,x0));
    let y0=drag.v.y0+dy/h*yS;y0=Math.max(0,Math.min(D.nB-yS,y0));
    view={x0,x1:x0+xS,y0,y1:y0+yS};render();
  }
  const r=cv.getBoundingClientRect();
  const col=e.clientX-r.left-FAW,row=e.clientY-r.top-MH;
  const w=vW(),h=vH();
  if(col>=0&&col<w&&row>=0&&row<h){
    const wi=Math.min(D.nW-1,Math.max(0,Math.floor(view.x0+col/w*(view.x1-view.x0))));
    const bi=Math.min(D.nB-1,Math.max(0,Math.floor(view.y0+(h-1-row)/h*(view.y1-view.y0))));
    document.getElementById('info').textContent=
      't = '+(wi*D.ws/D.sr).toFixed(3)+' s   |   f = '+(bi*D.hpb/1000).toFixed(2)+' kHz';
  }
});
window.addEventListener('mouseup',()=>drag=null);
cv.addEventListener('dblclick',()=>{view={x0:0,x1:D.nW,y0:0,y1:D.nB};render();});
document.querySelectorAll('tr[data-t0]').forEach(tr=>{
  tr.addEventListener('click',function(){
    const t0=+this.dataset.t0,t1=+this.dataset.t1;
    const s=t0*D.sr/D.ws,e=t1*D.sr/D.ws;
    const pad=Math.max(5,(e-s)*0.1)|0;
    view.x0=Math.max(0,s-pad);view.x1=Math.min(D.nW,e+pad);
    render();cv.scrollIntoView({behavior:'smooth'});
  });
});
window.addEventListener('resize',resize);
resize();
"#;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Encode `data` as base64, streaming directly to `w` (no intermediate String).
fn write_base64<W: std::io::Write>(w: &mut W, data: &[u8]) -> std::io::Result<()> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = [0u8; 4];
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        buf[0] = T[(n >> 18) as usize];
        buf[1] = T[((n >> 12) & 63) as usize];
        buf[2] = if chunk.len() > 1 { T[((n >> 6) & 63) as usize] } else { b'=' };
        buf[3] = if chunk.len() > 2 { T[(n & 63) as usize] } else { b'=' };
        w.write_all(&buf)?;
    }
    Ok(())
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── HTML writer ───────────────────────────────────────────────────────────────

/// Write a self-contained interactive HTML spectrogram viewer.
///
/// `spec_bytes` is window-major (`bytes[w * freq_bins + b]`), values 0–255.
pub fn write_html(
    stem: &str,
    sample_rate: f32,
    window_size: usize,
    n_windows: usize,
    freq_bins: usize,
    hz_per_bin: f32,
    spec_bytes: &[u8],
    detected: &[bool],
    calls: &[CallGroupInfo],
    passes: &[PassInfo],
) -> std::io::Result<()> {
    let out_path = format!("{}_spectrogram.html", stem);
    let file = std::fs::File::create(&out_path)?;
    let mut w = std::io::BufWriter::new(file);

    let duration_sec = n_windows as f32 * window_size as f32 / sample_rate;
    let n_pulses: usize = calls.iter().map(|c| c.peaks.len()).sum();

    // ── Head ─────────────────────────────────────────────────────────────────
    w.write_all(b"<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n")?;
    write!(w, "<title>Bat Spectrogram \u{2014} {}</title>\n", stem)?;
    w.write_all(HTML_CSS.as_bytes())?;
    w.write_all(b"</head>\n<body>\n")?;

    // ── Header ───────────────────────────────────────────────────────────────
    write!(w, "<h1>Bat Spectrogram \u{2014} {}</h1>\n", stem)?;
    write!(
        w,
        "<p class=\"meta\">Sample rate: {} Hz &nbsp;|&nbsp; \
         Duration: {:.1} s &nbsp;|&nbsp; \
         {} windows ({}-point FFT) &nbsp;|&nbsp; \
         {} pulse(s) &rarr; {} pass(es)</p>\n",
        sample_rate as u32, duration_sec, n_windows, window_size,
        n_pulses, passes.len(),
    )?;
    w.write_all(
        b"<p class=\"help\">Scroll: zoom time &nbsp;|&nbsp; \
          Shift+scroll: zoom frequency &nbsp;|&nbsp; \
          Drag: pan &nbsp;|&nbsp; \
          Double-click: reset view &nbsp;|&nbsp; \
          Click pass row to zoom</p>\n",
    )?;

    // ── Canvas ────────────────────────────────────────────────────────────────
    w.write_all(b"<div id=\"wrap\"><canvas id=\"cv\"></canvas></div>\n")?;
    w.write_all(b"<div id=\"info\">&nbsp;</div>\n")?;

    // ── Passes table ──────────────────────────────────────────────────────────
    w.write_all(b"<div id=\"calls\">\n")?;
    w.write_all(b"<h2>Species passes <span style=\"color:#555;font-size:11px\">(click to zoom)</span></h2>\n")?;
    w.write_all(
        b"<table><thead><tr>\
          <th>#</th><th>Time</th><th>Duration</th>\
          <th>Pulses</th><th>Mean peak</th><th>Species</th>\
          </tr></thead><tbody>\n",
    )?;

    for (i, pass) in passes.iter().enumerate() {
        let duration_ms = (pass.end_sec - pass.start_sec) * 1000.0;
        let row_class = if pass.dubious { "dubious" } else { "" };
        let pulses_cell = if pass.n_extra > 0 {
            format!(
                "{} <span style=\"color:#668;font-size:10px\">(+{}&nbsp;nearby)</span>",
                pass.n_pulses, pass.n_extra
            )
        } else {
            format!("{}", pass.n_pulses)
        };
        let species_cell = if pass.dubious {
            format!(
                "{} <span style=\"color:#555;font-size:10px\">(nested&nbsp;&#x2753;)</span>",
                pass.species
            )
        } else {
            pass.species.to_string()
        };
        write!(
            w,
            "<tr data-t0=\"{:.3}\" data-t1=\"{:.3}\" class=\"{}\">\
             <td>{}</td>\
             <td>{:.1}&ndash;{:.1}s</td>\
             <td>{:.0}ms</td>\
             <td>{}</td>\
             <td>{:.1}kHz</td>\
             <td>{}</td>\
             </tr>\n",
            pass.start_sec, pass.end_sec,
            row_class,
            i + 1,
            pass.start_sec, pass.end_sec,
            duration_ms,
            pulses_cell,
            pass.mean_peak_hz / 1000.0,
            species_cell,
        )?;
    }
    w.write_all(b"</tbody></table>\n</div>\n")?;

    // ── Script: dynamic data ──────────────────────────────────────────────────
    w.write_all(b"<script>\n")?;

    write!(
        w,
        "const D={{nW:{},nB:{},sr:{},ws:{},hpb:{:.6}}};\n",
        n_windows, freq_bins, sample_rate as u32, window_size, hz_per_bin
    )?;

    // Detected array (compact 0/1)
    w.write_all(b"D.det=[")?;
    for (i, &d) in detected.iter().enumerate() {
        if i > 0 { w.write_all(b",")?; }
        w.write_all(if d { b"1" } else { b"0" })?;
    }
    w.write_all(b"];\n")?;

    // Calls array — start/end windows, used only for boundary lines in the spectrogram
    w.write_all(b"D.calls=[")?;
    for (i, call) in calls.iter().enumerate() {
        if i > 0 { w.write_all(b",")?; }
        write!(w, r#"{{"s":{},"e":{}}}"#, call.start_win, call.end_win)?;
    }
    w.write_all(b"];\n")?;

    // Base64 spectrogram data → D.bytes
    w.write_all(b"(function(){const s='")?;
    write_base64(&mut w, spec_bytes)?;
    w.write_all(b"';const b=atob(s);const a=new Uint8Array(b.length);for(let i=0;i<b.length;i++)a[i]=b.charCodeAt(i);D.bytes=a;})();\n")?;

    // Static rendering + interaction code
    w.write_all(HTML_JS.as_bytes())?;
    w.write_all(b"</script>\n</body>\n</html>\n")?;

    println!("Interactive HTML saved to:  {}", out_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── colormap ──────────────────────────────────────────────────────────────

    #[test]
    fn colormap_black_at_zero() {
        assert_eq!(colormap(0.0), [0, 0, 0]);
    }

    #[test]
    fn colormap_red_at_one() {
        assert_eq!(colormap(1.0), [255, 0, 0]);
    }

    #[test]
    fn colormap_blue_at_quarter() {
        // At t=0.25 we are at the top of the blue ramp → full blue
        assert_eq!(colormap(0.25), [0, 0, 255]);
    }

    #[test]
    fn colormap_cyan_at_half() {
        // At t=0.5 we are at the top of the green ramp → cyan (0, 255, 255)
        assert_eq!(colormap(0.5), [0, 255, 255]);
    }

    #[test]
    fn colormap_yellow_at_three_quarters() {
        // At t=0.75 we are at the top of the red ramp → yellow (255, 255, 0)
        assert_eq!(colormap(0.75), [255, 255, 0]);
    }

    #[test]
    fn colormap_clamps_below_zero() {
        assert_eq!(colormap(-1.0), colormap(0.0));
    }

    #[test]
    fn colormap_clamps_above_one() {
        assert_eq!(colormap(2.0), colormap(1.0));
    }

    // ── write_base64 ──────────────────────────────────────────────────────────

    fn b64(data: &[u8]) -> String {
        let mut out = Vec::new();
        write_base64(&mut out, data).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn base64_empty() {
        assert_eq!(b64(b""), "");
    }

    #[test]
    fn base64_one_byte() {
        // 0x4d = 'M' → base64 "TQ=="
        assert_eq!(b64(b"M"), "TQ==");
    }

    #[test]
    fn base64_two_bytes() {
        // "Ma" → "TWE="
        assert_eq!(b64(b"Ma"), "TWE=");
    }

    #[test]
    fn base64_three_bytes_no_padding() {
        // "Man" → "TWFu"
        assert_eq!(b64(b"Man"), "TWFu");
    }

    #[test]
    fn base64_known_string() {
        // Standard test vector: "Hello" → "SGVsbG8="
        assert_eq!(b64(b"Hello"), "SGVsbG8=");
    }

    // ── html_escape ───────────────────────────────────────────────────────────

    #[test]
    fn html_escape_plain() {
        assert_eq!(html_escape("hello"), "hello");
    }

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("<b>\"foo\" & 'bar'</b>"), "&lt;b&gt;&quot;foo&quot; &amp; 'bar'&lt;/b&gt;");
    }
}
