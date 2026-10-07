// Glyph turn 2: the same three chromes, enchanted. Light is drawn with per-cell 24-bit colour ramps.
(function () {
'use strict';
const I = window.Griffin.internals, mix = window.GriffinThemes.mix;
const { len, cut, putRuns, putRight, dim, files, putName, LINES, CUR, DG, TOP, ROWS, TABS, FILE } = window.GlyphDirections.h;

const BASE = { bg: '#0b0a10', deep: '#07060b', surface: '#0f0e16', raised: '#16141f', raised2: '#1e1b2a', line2: '#2e2a40', guide: '#1f1c2b',
  muted: '#67627d', text: '#a6a2bb', fg: '#d2cfe2', strong: '#f4f2fb', err: '#ff6f91', warn: '#f0cf7a', ok: '#7fe3c9', info: '#8fd0ff',
  err_soft: '#b05a76', warn_soft: '#a38d58', info_soft: '#6a8db0', sel: '#221c35', cur_line: '#13111c', gutter: '#3a3550', scrim: '#000000' };
const SYN = { keyword: '#b69cff', string: '#7fe3c9', comment: '#5c5773', function: '#ece4ff', type: '#8fd0ff', number: '#f0cf7a', constant: '#f0cf7a',
  operator: '#7d7896', punctuation: '#5d5873', variable: '#d2cfe2', property: '#bab5d1', tag: '#b69cff', attribute: '#7d7896' };
function theme(o) { const r = Object.assign({}, BASE, o); delete r.syn; for (const k in SYN) r['syn.' + k] = (o.syn && o.syn[k]) || SYN[k]; return { r, mods: {}, mono: false }; }
const VIOLET = '#b69cff', TEAL = '#6ee7d8', MOON = '#a9c6ff', GOLD = '#f0cf7a';
const TH = {
  a: theme({ accent: VIOLET, accent2: TEAL, acc_ink: '#120a24' }),
  b: theme({ accent: TEAL, accent2: VIOLET, acc_ink: '#04201c' }),
  c: theme({ accent: MOON, accent2: GOLD, acc_ink: '#0b1426' }),
  m: theme({ accent: MOON, accent2: GOLD, acc_ink: '#0b1426', bg: '#090b10', deep: '#06080c', surface: '#0d1017', raised: '#141822', raised2: '#1b2030', line2: '#2a3246', guide: '#1a1f2b',
    muted: '#5f6a82', text: '#a2abbf', fg: '#d0d6e4', strong: '#f2f5fb', sel: '#1c2538', cur_line: '#10131b', gutter: '#343c50', info: MOON,
    syn: { keyword: MOON, string: '#e6d29c', comment: '#56607a', function: '#eef3ff', type: '#8fd8ff', number: GOLD, constant: GOLD, operator: '#77819a', punctuation: '#596379', variable: '#d0d6e4', property: '#c0c9dc', tag: MOON, attribute: '#77819a' } }),
};

const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
function grad(stops, t) { t = clamp(t, 0, 1); const n = stops.length - 1, i = Math.min(n - 1, Math.floor(t * n)), f = t * n - i; return mix(stops[i], stops[i + 1], f); }
function gradText(g, x, y, s, stops, b) { const a = [...s]; a.forEach((c, i) => g.set(x + i, y, c, { fg: grad(stops, i / Math.max(1, a.length - 1)), b: b ? 1 : 0 })); return x + a.length; }
function hash(x, y) { let h = (x * 374761393 + y * 668265263) ^ 0x5bd1e995; h = Math.imul(h ^ (h >>> 13), 1274126177); return ((h ^ (h >>> 16)) >>> 0) / 4294967296; }

// Editor body; the cursor line glows from the gutter and fades out to the right.
function code(g, r, x, y, w, h, o) {
  o = o || {};
  const D = 3, tx = x + D + 3, tw = w - D - 3, glow = [mix(r.bg, r.accent, .2), mix(r.bg, r.accent, .07), r.cur_line, r.cur_line];
  for (let k = 0; k < h; k++) {
    const li = TOP + k; if (li >= LINES.length) break;
    const yy = y + k, isCur = li === CUR.l;
    g.fill(x, yy, w, 1, { fg: 'fg', bg: 'bg' });
    if (isCur) for (let xx = x; xx < x + w; xx++) g.set(xx, yy, null, { bg: grad(glow, (xx - x) / (w * .8)) });
    const dl = DG.filter(d => d.l === li);
    if (dl.length) { const sev = dl.some(d => d.sev === 'err') ? 'err' : dl.some(d => d.sev === 'warn') ? 'warn' : 'info'; g.set(x, yy, '\u25c6', { fg: sev }); }
    g.put(x + 1, yy, String(li + 1).padStart(D), isCur ? { fg: r.accent, b: 1 } : { fg: 'gutter' });
    let col = 0;
    for (const [tok, role] of I.tokRust(LINES[li])) for (const ch of tok) {
      if (col < tw) { const st = { fg: role ? 'syn.' + role : 'fg' }; if (role === 'comment') st.i = 1; const d = dl.find(d => col >= d.c0 && col < d.c1); if (d) { st.u = 'c'; st.uc = d.sev; } g.set(tx + col, yy, ch, st); }
      col++;
    }
    if (o.lens && dl.length) {
      const d = dl.find(d => d.sev === 'err') || dl[0], lx = tx + LINES[li].length + 4;
      if (lx < x + w - 12) { g.put(lx, yy, '\u25c8 ', { fg: d.sev }); g.put(lx + 2, yy, cut(d.msg, x + w - lx - 4), { fg: d.sev + '_soft', i: 1 }); }
    }
    if (isCur) { const c = g.at(tx + CUR.c, yy); if (c) c.cur = 1; }
  }
}
function tree(g, r, x0, y0, w, yMax, o) {
  const glow = [mix(r.surface, r.accent, .26), mix(r.surface, r.accent, .08), r.surface];
  ROWS.forEach((row, i) => {
    const y = y0 + i; if (y >= yMax) return; const act = row.path === FILE;
    if (act) for (let x = x0; x < x0 + w; x++) g.set(x, y, null, { bg: grad(glow, (x - x0) / w) });
    if (o.guides) for (let d = 0; d < row.depth; d++) g.set(x0 + 3 + d * 2, y, '\u2502', { fg: 'guide' });
    let x = x0 + 2 + row.depth * 2;
    g.put(x, y, row.dir ? (row.open ? '\u25be' : '\u25b8') : ' ', { fg: 'muted' }); x += 2;
    g.put(x, y, cut(row.name, x0 + w - 3 - x), { fg: act ? 'strong' : 'text', b: act ? 1 : 0 });
    if (row.path === 'src/app.rs') g.set(x0 + w - 3, y, '\u2022', { fg: 'warn' });
  });
}
function glowRow(g, r, x0, x1, y, from, to, base) { for (let x = x0; x < x1; x++) g.set(x, y, null, { bg: grad([mix(base, from, .3), mix(base, to, .12), base], (x - x0) / (x1 - x0)) }); }

// 2a  Float, enchanted
function floatMain(W, H, t) {
  const r = (t || TH.a).r, g = new I.Grid(W, H), tw = 28; g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  g.fill(0, 0, tw, H - 1, { fg: 'text', bg: 'surface' });
  g.put(2, 1, '\u2726', { fg: r.accent2 }); gradText(g, 4, 1, 'glyph', [r.accent, r.accent2], true); g.put(12, 1, '~/src', { fg: 'muted' });
  tree(g, r, 0, 3, tw, H - 2, { guides: true });
  let x = tw + 2, pc = 0;
  for (const [n, act, dirty] of TABS) {
    const label = ' ' + n + (dirty ? ' \u2022' : '') + ' ';
    if (act) { g.set(x, 1, '\u2590', { fg: 'raised2', bg: 'bg' }); g.fill(x + 1, 1, len(label), 1, { bg: 'raised2' }); gradText(g, x + 2, 1, n, [r.strong, r.accent], true); g.set(x + 1 + len(label), 1, '\u258c', { fg: 'raised2', bg: 'bg' }); pc = x + 1 + len(label) / 2; }
    else g.put(x + 1, 1, label, { fg: 'muted' });
    if (dirty) g.set(x + 2 + len(n) + 1, 1, null, { fg: 'warn' });
    x += len(label) + 3;
  }
  for (let xx = tw + 1; xx < W; xx++) { const d = Math.abs(xx - pc); g.set(xx, 2, '\u2500', { fg: mix(grad([r.accent, r.accent2], (xx - tw) / (W - tw)), r.bg, clamp(d / 46, 0, 1) * .92 + .04) }); }
  code(g, r, tw + 1, 3, W - tw - 2, H - 5, { lens: true });
  g.fill(0, H - 1, W, 1, { fg: 'text', bg: 'surface' });
  for (let i = 0; i < 18; i++) g.set(i, H - 1, null, { bg: i < 10 ? grad([r.accent, r.accent2], i / 9) : mix(r.accent2, r.surface, (i - 9) / 9) });
  g.put(1, H - 1, '\u2726 glyph', { fg: 'acc_ink', b: 1 });
  putRuns(g, 20, H - 1, [['src/ui/', 'muted'], ['status.rs', 'strong']]);
  putRight(g, W - 2, H - 1, [['Ln 52, Col 22', 'text'], ['    '], ['Rust', 'text'], ['    '], ['\u25cf ', 'ok'], ['rust-analyzer', 'text'], ['    '], ['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn']]);
  return g;
}
function floatPalette(W, H, t) {
  const r = (t || TH.a).r, g = floatMain(W, H, t); dim(g);
  const w = 86, x = Math.floor((W - w) / 2), y = 4, q = 'stat';
  const fl = files(q, 5), cmds = [['Start run\u2026', 'F5'], ['Split right', 'Alt+V'], ['Toggle status line', '']].map(([l, k]) => ({ l, k, m: I.fuzzy(q, l) })).filter(c => c.m).slice(0, 2);
  const h = 5 + fl.length + 2 + cmds.length + 3;
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  for (let xx = x; xx < x + w; xx++) g.set(xx, y, '\u2580', { fg: grad([r.accent, r.accent2, r.accent], (xx - x) / (w - 1)), bg: 'raised' });
  g.put(x + 4, y + 2, '\u2726', { fg: r.accent2, b: 1 }); const e = g.put(x + 6, y + 2, q, { fg: 'strong' }); g.at(e, y + 2).cur = 1;
  putRight(g, x + w - 4, y + 2, [['cast', r.accent], [' \u00b7 files \u00b7 commands', 'muted']]);
  for (let xx = x + 2; xx < x + w - 2; xx++) g.set(xx, y + 3, '\u2500', { fg: 'line2' });
  let yy = y + 4; g.put(x + 4, yy++, 'FILES', { fg: 'muted', b: 1 });
  fl.forEach((f, i) => { const on = i === 0; if (on) glowRow(g, r, x, x + w, yy, r.accent, r.accent2, r.raised); const ex = putName(g, x + 4, yy, f.name, f.idx, on); g.put(ex + 2, yy, f.dir, { fg: 'muted' }); if (on) putRight(g, x + w - 4, yy, [['\u23ce', r.accent]]); yy++; });
  yy++; g.put(x + 4, yy++, 'COMMANDS', { fg: 'muted', b: 1 });
  cmds.forEach(c => { putName(g, x + 4, yy, c.l, c.m.idx, false); if (c.k) putRight(g, x + w - 4, yy, [[c.k, 'muted']]); yy++; });
  yy++; putRuns(g, x + 4, yy, [['>', r.accent, 1], [' commands   ', 'muted'], [':', r.accent, 1], [' line   ', 'muted'], ['@', r.accent, 1], [' symbol   ', 'muted'], ['/', r.accent, 1], [' text', 'muted']]);
  putRight(g, x + w - 4, yy, [['\u2191\u2193  \u23ce  esc', 'muted']]);
  return g;
}

// 2b  Rail, enchanted
function railMain(W, H) {
  const r = TH.b.r, g = new I.Grid(W, H), rw = 4, tw = 28, ex = rw + tw; g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  g.fill(0, 0, rw, H - 1, { fg: 'muted', bg: 'deep' });
  for (let x = 0; x < rw; x++) g.set(x, 1, null, { bg: mix(r.deep, r.accent, .18 - x * .04) });
  [['\u25c8', 1], ['\u2727', 3], ['\u25b7', 5], ['\u25c7', 7]].forEach(([c, y], i) => g.set(1, y, c, { fg: i === 0 ? r.accent : 'muted', b: i === 0 ? 1 : 0 }));
  g.set(1, H - 3, '\u2726', { fg: mix(r.deep, r.accent2, .5) });
  g.fill(rw, 0, tw, H - 1, { fg: 'text', bg: 'surface' });
  g.put(rw + 2, 0, 'FILES', { fg: 'muted', b: 1 });
  tree(g, r, rw, 2, tw, H - 2, { guides: false });
  g.fill(ex, 0, W - ex, 1, { fg: 'muted', bg: 'surface' });
  let x = ex;
  for (const [n, act, dirty] of TABS) {
    const label = '  ' + n + (dirty ? ' \u2022' : '') + '  ';
    if (act) { g.fill(x, 0, len(label), 1, { bg: 'bg' }); gradText(g, x + 2, 0, n, [r.accent, r.accent2], true); for (let i = 0; i < len(label); i++) g.set(x + i, 1, '\u2594', { fg: grad([r.accent, r.accent2], i / (len(label) - 1)) }); }
    else g.put(x, 0, label, { fg: 'muted' });
    if (dirty) g.set(x + 3 + len(n), 0, null, { fg: 'warn' });
    x += len(label);
  }
  putRuns(g, x + 2, 1, []);
  const bx = ex + 2 + 22;
  putRuns(g, bx, 1, [['src', 'muted'], [' \u203a ', 'gutter'], ['ui', 'muted'], [' \u203a ', 'gutter'], ['status.rs', 'text'], [' \u203a ', 'gutter'], ['\u0192 ', r.accent], ['render_status', 'strong']]);
  code(g, r, ex, 3, W - ex, H - 4);
  g.fill(0, H - 1, W, 1, { fg: 'text', bg: 'surface' });
  for (let i = 0; i < 18; i++) g.set(i, H - 1, null, { bg: i < 10 ? grad([r.accent, r.accent2], i / 9) : mix(r.accent2, r.surface, (i - 9) / 9) });
  g.put(1, H - 1, '\u2726 glyph', { fg: 'acc_ink', b: 1 });
  putRuns(g, 20, H - 1, [['\u25c8 ', 'err'], ['mismatched types: expected u16, found usize', 'text']]);
  putRight(g, W - 2, H - 1, [['Ln 52, Col 22', 'text'], ['    '], ['Rust', 'text'], ['    '], ['\u25cf ', 'ok'], ['rust-analyzer', 'text'], ['    '], ['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn']]);
  return g;
}
function railPalette(W, H) {
  const r = TH.b.r, g = railMain(W, H); dim(g);
  const w = 90, h = 21, x = Math.floor((W - w) / 2), y = Math.floor((H - h) / 2), q = 'stat';
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  g.fill(x, y, w, 1, { bg: 'raised2' });
  let cx = x + 1;
  ['Files', 'Symbols', 'Lines', 'Spells', 'Search'].forEach((t, i) => {
    const s = ' ' + t + ' ';
    if (i === 0) { for (let j = 0; j < len(s); j++) g.set(cx + j, y, s[j], { fg: 'acc_ink', bg: grad([r.accent, r.accent2], j / (len(s) - 1)), b: 1 }); cx += len(s) + 1; }
    else cx = g.put(cx, y, s, { fg: 'muted' }) + 1;
  });
  putRight(g, x + w - 2, y, [['tab switches', 'muted']]);
  g.put(x + 2, y + 2, '\u2726', { fg: r.accent, b: 1 }); const e = g.put(x + 4, y + 2, q, { fg: 'strong' }); g.at(e, y + 2).cur = 1;
  putRight(g, x + w - 2, y + 2, [[files(q, 99).length + ' of 49', 'muted']]);
  files(q, h - 6).forEach((f, i) => { const yy = y + 4 + i, on = i === 0; if (on) glowRow(g, r, x, x + w, yy, r.accent, r.accent2, r.raised); putName(g, x + 2, yy, f.name, f.idx, on); putRight(g, x + w - 2, yy, [[f.dir, on ? 'text' : 'muted']]); });
  putRuns(g, x + 2, y + h - 1, [['\u2191\u2193 select   \u23ce open   ctrl+\u23ce open beside   esc close', 'muted']]);
  return g;
}

// 2c  Zen, starlit
function stars(g, r, W, H, keep) {
  for (let y = 1; y < H - 1; y++) for (let x = 0; x < W; x++) {
    if (keep(x, y)) continue;
    const v = hash(x, y);
    if (v < .0035) g.set(x, y, '\u2726', { fg: mix(r.bg, r.accent2, .55) });
    else if (v < .009) g.set(x, y, '\u22c6', { fg: mix(r.bg, r.accent, .4) });
    else if (v < .03) g.set(x, y, '\u00b7', { fg: mix(r.bg, r.accent, .18 + hash(y, x) * .2) });
  }
}
function zenMain(W, H) {
  const r = TH.c.r, g = new I.Grid(W, H); g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  const cw = Math.min(104, W - 4), cx = Math.floor((W - cw) / 2);
  stars(g, r, W, H, (x, y) => x >= cx - 3 && x < cx + cw + 3);
  g.put(2, 0, '\u2726', { fg: r.accent2 }); gradText(g, 4, 0, 'glyph', [r.accent, r.accent2], true);
  let x = cx;
  for (const [n, act, dirty] of TABS) {
    if (act) { g.put(x, 0, '\u2726 ', { fg: r.accent2 }); x = g.put(x + 2, 0, n, { fg: 'strong', b: 1 }); }
    else { x = g.put(x, 0, n, { fg: 'muted' }); if (dirty) x = g.put(x, 0, ' \u2022', { fg: 'warn' }); }
    x += 4;
  }
  putRight(g, W - 2, 0, [['glyph', 'muted'], [' / ', 'gutter'], ['src', 'muted'], [' / ', 'gutter'], ['ui', 'muted']]);
  for (let xx = cx; xx < cx + cw; xx++) g.set(xx, 1, '\u2500', { fg: mix(grad([r.accent, r.accent2, r.accent], (xx - cx) / cw), r.bg, .78) });
  code(g, r, cx, 3, cw, H - 5);
  putRuns(g, 2, H - 1, [['\u25c8 ', 'err'], ['mismatched types: expected u16, found usize', 'muted']]);
  putRight(g, W - 2, H - 1, [['52', 'text'], ['\u00b7', r.accent2], ['22', 'text'], ['    '], ['rust ', 'muted'], ['\u25cf', 'ok'], ['    '], ['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn']]);
  return g;
}
function zenPalette(W, H) {
  const r = TH.c.r, g = zenMain(W, H); dim(g);
  const w = 74, q = 'stat', fl = files(q, 7), h = fl.length + 6, x = Math.floor((W - w) / 2), y = 7;
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  const per = 2 * (w + h), stops = [r.accent, r.accent2, r.accent, '#b69cff', r.accent];
  const at = (i) => ({ fg: grad(stops, i / per), bg: 'raised' });
  for (let i = 0; i < w; i++) { g.set(x + i, y, '\u2500', at(i)); g.set(x + w - 1 - i, y + h - 1, '\u2500', at(w + h + i)); }
  for (let j = 0; j < h; j++) { g.set(x + w - 1, y + j, '\u2502', at(w + j)); g.set(x, y + h - 1 - j, '\u2502', at(2 * w + h + j)); }
  g.set(x, y, '\u256d', at(0)); g.set(x + w - 1, y, '\u256e', at(w)); g.set(x + w - 1, y + h - 1, '\u256f', at(w + h)); g.set(x, y + h - 1, '\u2570', at(2 * w + h));
  putRuns(g, x + 2, y, [[' ', 'muted'], ['\u2726', r.accent2], [' cast ', 'strong'], [' ', 'muted']]);
  g.put(x + 3, y + 2, '\u203a', { fg: r.accent2, b: 1 }); const e = g.put(x + 5, y + 2, q, { fg: 'strong' }); g.at(e, y + 2).cur = 1;
  fl.forEach((f, i) => { const yy = y + 4 + i, on = i === 0; if (on) glowRow(g, r, x + 1, x + w - 1, yy, r.accent, r.accent2, r.raised); putName(g, x + 3, yy, f.name, f.idx, on); putRight(g, x + w - 3, yy, [[f.dir, on ? 'text' : 'muted']]); });
  putRuns(g, x + 2, y + h - 1, [[' ', 'muted'], ['>', r.accent2, 1], [' spells \u00b7 ', 'muted'], [':', r.accent2, 1], [' line \u00b7 ', 'muted'], ['@', r.accent2, 1], [' symbol \u00b7 ', 'muted'], ['/', r.accent2, 1], [' text ', 'muted']]);
  return g;
}

function rgbD(a, b) { const p = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16)); const x = p(a), y = p(b); return Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]); }
function fromHydra(name) {
  const h = window.GriffinThemes.theme(name).r, dark = window.GriffinThemes.lum(h.bg) < .5;
  const accent2 = ['syn.type', 'syn.function', 'syn.string', 'syn.keyword'].map(k => h[k]).sort((a, b) => rgbD(b, h.accent) - rgbD(a, h.accent))[0];
  const r = { bg: h.bg, deep: mix(h.bg, dark ? '#000000' : h.fg, .15), surface: h.sidebar_bg, raised: h.card, raised2: h.card2, line2: h.border, guide: mix(h.sidebar_bg, h.line, .6),
    muted: h.muted, text: h.text, fg: h.fg, strong: h.strong, accent: h.accent, accent2, acc_ink: h.acc_ink,
    err: h.err, warn: h.working, ok: h.done, info: h.info, err_soft: mix(h.err, h.bg, .35), warn_soft: mix(h.working, h.bg, .35), info_soft: mix(h.info, h.bg, .35),
    sel: h.selection_bg, cur_line: h.current_line_bg, gutter: h.gutter_fg, scrim: h.scrim };
  for (const k in h) if (k.startsWith('syn.')) r[k] = h[k];
  return { name, r, mods: {}, mono: false };
}
window.GlyphMagic = { fromHydra, floatMain, floatPalette, TH, frames: { 'a2-main': [floatMain, 'a'], 'a2-pal': [floatPalette, 'a'], 'b2-main': [railMain, 'b'], 'b2-pal': [railPalette, 'b'], 'c2-main': [zenMain, 'c'], 'c2-pal': [zenPalette, 'c'] } };
})();
