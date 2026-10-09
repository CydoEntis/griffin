// Glyph: three chrome directions on the Obsidian palette. Static frames at any W×H.
(function () {
'use strict';
const I = window.Griffin.internals, C = window.GriffinContent;
const len = s => [...s].length;
const cut = (s, n) => len(s) <= n ? s : [...s].slice(0, Math.max(0, n - 1)).join('') + '\u2026';

const R = {
  bg: '#0b0c0e', deep: '#08090a', surface: '#0f1113', raised: '#15181b', raised2: '#1c2024', line2: '#2a3036', guide: '#1d2125',
  muted: '#5d6670', text: '#9aa3ac', fg: '#c9cfd6', strong: '#eef1f4', accent: '#8cc8ff', acc_ink: '#06121e',
  err: '#ff6e7f', warn: '#e8c37a', ok: '#86d6a8', info: '#8cc8ff',
  err_soft: '#a8505b', warn_soft: '#9a8256', info_soft: '#5f86a8',
  sel: '#1a2a3c', cur_line: '#111316', gutter: '#30363c', gutter_active: '#eef1f4', scrim: '#000000',
};
const SYN = { keyword: '#8cc8ff', string: '#a9d8c4', comment: '#4b535c', function: '#e9edf1', type: '#b8c4d8', number: '#cdbff0', constant: '#cdbff0',
  operator: '#6c7581', punctuation: '#555d66', variable: '#c9cfd6', property: '#aeb7c1', tag: '#8cc8ff', attribute: '#6c7581' };
for (const k in SYN) R['syn.' + k] = SYN[k];
const THEME = { name: 'obsidian', r: R, mods: {}, mono: false };

const FILE = 'src/ui/status.rs', LINES = C.FILES[FILE].split('\n');
const CUR = { l: LINES.findIndex(l => l.includes('area.width as usize')), c: 21 };
const DG = C.DIAGS.filter(d => d[0] === FILE).map(d => { const l = LINES.findIndex(x => x.includes(d[1])); const c0 = LINES[l].indexOf(d[2]); return { l, c0, c1: c0 + d[2].length, sev: d[3], msg: d[4] }; });
const TOP = Math.max(0, CUR.l - 14);
const ROWS = I.treeRows({ tree: { open: new Set(['src', 'src/ui']) }, project: 'griffin' });
const TABS = [['app.rs', false, true], ['status.rs', true, false], ['theme.rs', false, false], ['mod.rs', false, false]];

function code(g, x, y, w, h, o) {
  o = o || {};
  const D = 3, tx = x + D + 3, tw = w - D - 3;
  for (let k = 0; k < h; k++) {
    const li = TOP + k; if (li >= LINES.length) break;
    const yy = y + k, isCur = li === CUR.l, bg = isCur ? 'cur_line' : 'bg';
    g.fill(x, yy, w, 1, { fg: 'fg', bg });
    const dl = DG.filter(d => d.l === li);
    if (dl.length) { const sev = dl.some(d => d.sev === 'err') ? 'err' : dl.some(d => d.sev === 'warn') ? 'warn' : 'info'; g.set(x, yy, '\u25cf', { fg: sev }); }
    g.put(x + 1, yy, String(li + 1).padStart(D), isCur ? { fg: 'gutter_active', b: 1 } : { fg: 'gutter' });
    let col = 0;
    for (const [tok, role] of I.tokRust(LINES[li])) for (const ch of tok) {
      if (col < tw) { const st = { fg: role ? 'syn.' + role : 'fg' }; if (role === 'comment') st.i = 1; const d = dl.find(d => col >= d.c0 && col < d.c1); if (d) { st.u = 'c'; st.uc = d.sev; } g.set(tx + col, yy, ch, st); }
      col++;
    }
    if (o.lens && dl.length) {
      const d = dl.find(d => d.sev === 'err') || dl[0], lx = tx + LINES[li].length + 4;
      if (lx < x + w - 12) { g.put(lx, yy, '\u25c6 ', { fg: d.sev }); g.put(lx + 2, yy, cut(d.msg, x + w - lx - 4), { fg: d.sev + '_soft', i: 1 }); }
    }
    if (isCur) { const c = g.at(tx + CUR.c, yy); if (c) c.cur = 1; }
  }
}
function putRuns(g, x, y, runs) { for (const [t, fg, b] of runs) x = g.put(x, y, t, { fg: fg || 'text', b: b ? 1 : 0 }); return x; }
const runsW = runs => runs.reduce((a, r) => a + len(r[0]), 0);
function putRight(g, xr, y, runs) { putRuns(g, xr - runsW(runs), y, runs); }
function dim(g) { for (const c of g.cells) { c.dim = 1; c.cur = 0; } }
function matchName(path, q) {
  const m = I.fuzzy(q, path); const base = path.lastIndexOf('/') + 1;
  return { name: path.slice(base), dir: path.slice(0, Math.max(0, base - 1)), idx: m ? m.idx.filter(i => i >= base).map(i => i - base) : [], score: m ? m.score : -1 };
}
function files(q, n) { return C.PATHS.map(p => matchName(p, q)).filter(m => m.score >= 0 && m.idx.length).sort((a, b) => b.score - a.score).slice(0, n); }
function putName(g, x, y, name, idx, on) { [...name].forEach((ch, i) => g.set(x + i, y, ch, idx.includes(i) ? { fg: 'accent', b: 1 } : { fg: on ? 'strong' : 'fg', b: 0 })); return x + len(name); }
function tree(g, x0, y0, w, yMax, o) {
  ROWS.forEach((r, i) => {
    const y = y0 + i; if (y >= yMax) return; const act = r.path === FILE;
    if (act) g.fill(x0 + 1, y, w - 2, 1, { bg: o.selBg });
    if (o.guides) for (let d = 0; d < r.depth; d++) g.set(x0 + 3 + d * 2, y, '\u2502', { fg: 'guide' });
    let x = x0 + 2 + r.depth * 2;
    g.put(x, y, r.dir ? (r.open ? '\u25be' : '\u25b8') : ' ', { fg: 'muted' }); x += 2;
    g.put(x, y, cut(r.name, x0 + w - 3 - x), { fg: act ? 'strong' : 'text', b: act ? 1 : 0 });
    if (r.path === 'src/app.rs') g.set(x0 + w - 3, y, '\u2022', { fg: 'warn' });
  });
}

// 1a  Float
function floatMain(W, H) {
  const g = new I.Grid(W, H), tw = 28; g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  g.fill(0, 0, tw, H - 1, { fg: 'text', bg: 'surface' });
  g.put(2, 1, '\u25c6', { fg: 'accent' }); g.put(4, 1, 'glyph', { fg: 'strong', b: 1 }); g.put(12, 1, '~/src', { fg: 'muted' });
  tree(g, 0, 3, tw, H - 2, { selBg: 'raised2', guides: true });
  let x = tw + 2;
  for (const [n, act, dirty] of TABS) {
    const label = ' ' + n + (dirty ? ' \u2022' : '') + ' ';
    if (act) { g.set(x, 1, '\u2590', { fg: 'raised2', bg: 'bg' }); g.put(x + 1, 1, label, { fg: 'strong', bg: 'raised2', b: 1 }); g.set(x + 1 + len(label), 1, '\u258c', { fg: 'raised2', bg: 'bg' }); }
    else g.put(x + 1, 1, label, { fg: 'muted' });
    if (dirty) g.set(x + 2 + len(n) + 1, 1, null, { fg: 'warn' });
    x += len(label) + 3;
  }
  code(g, tw + 1, 3, W - tw - 2, H - 5, { lens: true });
  g.put(2, H - 1, '\u25c6', { fg: 'accent' }); putRuns(g, 4, H - 1, [['src/ui/', 'muted'], ['status.rs', 'strong']]);
  putRight(g, W - 2, H - 1, [['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn'], ['      '], ['rust-analyzer', 'muted'], [' \u25cf', 'ok'], ['      '], ['52', 'strong'], [':', 'muted'], ['22', 'strong']]);
  return g;
}
function floatPalette(W, H) {
  const g = floatMain(W, H); dim(g);
  const w = 84, x = Math.floor((W - w) / 2), y = 3, q = 'stat';
  const fl = files(q, 5), cmds = [['Start run\u2026', 'F5'], ['Split right', 'Alt+V'], ['Toggle status line', '']].map(([l, k]) => ({ l, k, m: I.fuzzy(q, l) })).filter(c => c.m).slice(0, 2);
  const h = 4 + 1 + fl.length + 2 + cmds.length + 3;
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  g.fill(x + 2, y + 1, w - 4, 1, { bg: 'raised2' });
  g.put(x + 4, y + 1, '\u203a', { fg: 'accent', b: 1 }); const e = g.put(x + 6, y + 1, q, { fg: 'strong' }); g.at(e, y + 1).cur = 1;
  putRight(g, x + w - 4, y + 1, [['files \u00b7 commands', 'muted']]);
  let yy = y + 3; g.put(x + 4, yy++, 'FILES', { fg: 'muted', b: 1 });
  fl.forEach((f, i) => { const on = i === 0; if (on) g.fill(x + 2, yy, w - 4, 1, { bg: 'sel' }); const ex = putName(g, x + 4, yy, f.name, f.idx, on); g.put(ex + 2, yy, f.dir, { fg: 'muted' }); if (on) putRight(g, x + w - 4, yy, [['\u23ce', 'muted']]); yy++; });
  yy++; g.put(x + 4, yy++, 'COMMANDS', { fg: 'muted', b: 1 });
  cmds.forEach(c => { putName(g, x + 4, yy, c.l, c.m.idx, false); if (c.k) putRight(g, x + w - 4, yy, [[c.k, 'muted']]); yy++; });
  yy++; putRuns(g, x + 4, yy, [['>', 'strong', 1], [' commands   ', 'muted'], [':', 'strong', 1], [' line   ', 'muted'], ['@', 'strong', 1], [' symbol   ', 'muted'], ['/', 'strong', 1], [' text', 'muted']]);
  putRight(g, x + w - 4, yy, [['\u2191\u2193  \u23ce  esc', 'muted']]);
  return g;
}

// 1b  Rail
function railMain(W, H) {
  const g = new I.Grid(W, H), rw = 4, tw = 28, ex = rw + tw; g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  g.fill(0, 0, rw, H - 1, { fg: 'muted', bg: 'deep' });
  [['\u2261', 1], ['/', 3], ['\u25b7', 5], ['!', 7]].forEach(([c, y], i) => g.set(1, y, c, { fg: i === 0 ? 'accent' : 'muted', b: i === 0 ? 1 : 0 }));
  g.set(1, H - 3, '\u00b7', { fg: 'muted' });
  g.fill(rw, 0, tw, H - 1, { fg: 'text', bg: 'surface' });
  g.put(rw + 2, 0, 'FILES', { fg: 'muted', b: 1 });
  tree(g, rw, 2, tw, H - 2, { selBg: 'raised2', guides: false });
  g.fill(ex, 0, W - ex, 1, { fg: 'muted', bg: 'surface' });
  let x = ex;
  for (const [n, act, dirty] of TABS) {
    const label = '  ' + n + (dirty ? ' \u2022' : '') + '  ';
    g.put(x, 0, label, act ? { fg: 'strong', bg: 'bg', b: 1 } : { fg: 'muted' });
    if (dirty) g.set(x + 3 + len(n), 0, null, { fg: 'warn' });
    x += len(label);
  }
  putRuns(g, ex + 2, 1, [['src', 'muted'], [' \u203a ', 'gutter'], ['ui', 'muted'], [' \u203a ', 'gutter'], ['status.rs', 'text'], [' \u203a ', 'gutter'], ['\u0192 ', 'syn.function'], ['render_status', 'strong']]);
  code(g, ex, 3, W - ex, H - 4);
  g.fill(0, H - 1, W, 1, { fg: 'text', bg: 'surface' });
  g.put(0, H - 1, ' \u25c6 glyph ', { fg: 'acc_ink', bg: 'accent', b: 1 });
  putRuns(g, 11, H - 1, [['\u2715 ', 'err'], ['mismatched types: expected u16, found usize', 'text']]);
  putRight(g, W - 2, H - 1, [['Ln 52, Col 22', 'text'], ['    '], ['Rust', 'text'], ['    '], ['\u25cf ', 'ok'], ['rust-analyzer', 'text'], ['    '], ['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn']]);
  return g;
}
function railPalette(W, H) {
  const g = railMain(W, H); dim(g);
  const w = 88, h = 20, x = Math.floor((W - w) / 2), y = Math.floor((H - h) / 2), q = 'stat';
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  g.fill(x, y, w, 1, { bg: 'raised2' });
  let cx = x + 1;
  ['Files', 'Symbols', 'Lines', 'Commands', 'Search'].forEach((t, i) => { cx = g.put(cx, y, ' ' + t + ' ', i === 0 ? { fg: 'acc_ink', bg: 'accent', b: 1 } : { fg: 'muted' }) + 1; });
  putRight(g, x + w - 2, y, [['tab switches', 'muted']]);
  g.put(x + 2, y + 2, '\u203a', { fg: 'accent', b: 1 }); const e = g.put(x + 4, y + 2, q, { fg: 'strong' }); g.at(e, y + 2).cur = 1;
  putRight(g, x + w - 2, y + 2, [[files(q, 99).length + ' of ' + C.PATHS.length, 'muted']]);
  files(q, h - 6).forEach((f, i) => { const yy = y + 4 + i, on = i === 0; if (on) g.fill(x, yy, w, 1, { bg: 'sel' }); putName(g, x + 2, yy, f.name, f.idx, on); putRight(g, x + w - 2, yy, [[f.dir, on ? 'text' : 'muted']]); });
  putRuns(g, x + 2, y + h - 1, [['\u2191\u2193 select   \u23ce open   ctrl+\u23ce open to the side   esc close', 'muted']]);
  return g;
}

// 1c  Zen
function zenMain(W, H) {
  const g = new I.Grid(W, H); g.fill(0, 0, W, H, { fg: 'fg', bg: 'bg' });
  const cw = Math.min(104, W - 4), cx = Math.floor((W - cw) / 2);
  g.put(2, 0, '\u25c6', { fg: 'accent' }); g.put(4, 0, 'glyph', { fg: 'muted' });
  let x = cx;
  for (const [n, act, dirty] of TABS) {
    if (act) { g.put(x, 0, '\u25c6 ', { fg: 'accent' }); x = g.put(x + 2, 0, n, { fg: 'strong', b: 1 }); }
    else { x = g.put(x, 0, n, { fg: 'muted' }); if (dirty) x = g.put(x, 0, ' \u2022', { fg: 'warn' }); }
    x += 4;
  }
  putRight(g, W - 2, 0, [['glyph', 'muted'], [' / ', 'gutter'], ['src', 'muted'], [' / ', 'gutter'], ['ui', 'muted']]);
  code(g, cx, 3, cw, H - 5);
  putRuns(g, 2, H - 1, [['\u2715 ', 'err'], ['mismatched types: expected u16, found usize', 'muted']]);
  putRight(g, W - 2, H - 1, [['52:22', 'text'], ['    '], ['rust ', 'muted'], ['\u25cf', 'ok'], ['    '], ['\u2715 2', 'err'], ['  '], ['\u26a0 1', 'warn']]);
  return g;
}
function zenPalette(W, H) {
  const g = zenMain(W, H); dim(g);
  const w = 72, q = 'stat', fl = files(q, 7), h = fl.length + 6, x = Math.floor((W - w) / 2), y = 7;
  g.fill(x, y, w, h, { fg: 'text', bg: 'raised' });
  const b = { fg: 'line2', bg: 'raised' };
  for (let i = x + 1; i < x + w - 1; i++) { g.set(i, y, '\u2500', b); g.set(i, y + h - 1, '\u2500', b); }
  for (let j = y + 1; j < y + h - 1; j++) { g.set(x, j, '\u2502', b); g.set(x + w - 1, j, '\u2502', b); }
  g.set(x, y, '\u256d', b); g.set(x + w - 1, y, '\u256e', b); g.set(x, y + h - 1, '\u2570', b); g.set(x + w - 1, y + h - 1, '\u256f', b);
  putRuns(g, x + 2, y, [[' ', 'muted'], ['\u25c6', 'accent'], [' go to ', 'text'], [' ', 'muted']]);
  g.put(x + 3, y + 2, '\u203a', { fg: 'accent', b: 1 }); const e = g.put(x + 5, y + 2, q, { fg: 'strong' }); g.at(e, y + 2).cur = 1;
  fl.forEach((f, i) => { const yy = y + 4 + i, on = i === 0; if (on) g.fill(x + 1, yy, w - 2, 1, { bg: 'sel' }); putName(g, x + 3, yy, f.name, f.idx, on); putRight(g, x + w - 3, yy, [[f.dir, on ? 'text' : 'muted']]); });
  putRuns(g, x + 2, y + h - 1, [[' ', 'muted'], ['>', 'text', 1], [' cmd \u00b7 ', 'muted'], [':', 'text', 1], [' line \u00b7 ', 'muted'], ['@', 'text', 1], [' symbol \u00b7 ', 'muted'], ['/', 'text', 1], [' text ', 'muted']]);
  return g;
}

window.GlyphDirections = { h: { len, cut, putRuns, putRight, dim, files, putName, LINES, CUR, DG, TOP, ROWS, TABS, FILE }, THEME, frames: { 'a-main': floatMain, 'a-pal': floatPalette, 'b-main': railMain, 'b-pal': railPalette, 'c-main': zenMain, 'c-pal': zenPalette } };
})();
