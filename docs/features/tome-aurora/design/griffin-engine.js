// Griffin cell-grid engine: layout, rendering by theme role, keys, mouse, scenarios.
(function () {
'use strict';
const T = window.GriffinThemes, C = window.GriffinContent;
const { clamp, mix } = T;

// ---------- grid
const BASE = { b: 0, d: 0, i: 0, u: 0, uc: null, dim: 0, cur: 0 };
class Grid {
  constructor(W, H) { this.W = W; this.H = H; this.cells = []; this.hits = []; this.region = 'screen'; this.cursor = null;
    for (let i = 0; i < W * H; i++) this.cells.push(Object.assign({ c: ' ', fg: 'fg', bg: 'bg', reg: 'screen' }, BASE)); }
  at(x, y) { return (x < 0 || y < 0 || x >= this.W || y >= this.H) ? null : this.cells[y * this.W + x]; }
  set(x, y, ch, st) { const c = this.at(x, y); if (!c) return; if (ch != null) c.c = ch; if (st) Object.assign(c, st); c.reg = this.region; }
  put(x, y, s, st, maxX) { let i = 0; for (const ch of s) { if (maxX != null && x + i >= maxX) break; this.set(x + i, y, ch, st); i++; } return x + i; }
  fill(x, y, w, h, st) { for (let yy = y; yy < y + h; yy++) for (let xx = x; xx < x + w; xx++) { const c = this.at(xx, yy); if (!c) continue; Object.assign(c, BASE, { c: ' ' }, st); c.reg = this.region; } }
  hit(x, y, w, h, o) { this.hits.push(Object.assign({ x, y, w, h }, o)); }
  find(x, y) { for (let i = this.hits.length - 1; i >= 0; i--) { const h = this.hits[i]; if (x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h) return h; } return null; }
}
const len = s => [...s].length;
const padEnd = (s, n) => s + ' '.repeat(Math.max(0, n - len(s)));
const cut = (s, n) => len(s) <= n ? s : [...s].slice(0, Math.max(0, n - 1)).join('') + '\u2026';

// ---------- tokenizers
const KW = new Set('as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while'.split(' '));
function tokRust(line) {
  const out = []; const re = /(\/\/.*$)|("(?:[^"\\]|\\.)*"?)|('(?:[^'\\]|\\.)')|('[a-z_]+)|(#!?\[[^\]]*\]?)|(\b\d[\d_]*(?:\.\d+)?\b)|([A-Za-z_][A-Za-z0-9_]*)|(::|->|=>|==|!=|<=|>=|&&|\|\||[+\-*\/%=<>!&|^?@])|([{}()\[\],;.:])|(\s+)|(.)/g;
  let m;
  while ((m = re.exec(line))) {
    const t = m[0]; let role = null;
    if (m[1]) role = 'comment'; else if (m[2] || m[3]) role = 'string'; else if (m[4]) role = 'keyword'; else if (m[5]) role = 'attribute'; else if (m[6]) role = 'number';
    else if (m[7]) {
      const rest = line.slice(re.lastIndex), prev = line.slice(0, m.index);
      if (t === 'true' || t === 'false') role = 'constant';
      else if (KW.has(t)) role = 'keyword';
      else if (rest[0] === '!' || /^\s*\(/.test(rest) || /^::</.test(rest)) role = 'function';
      else if (/^[A-Z][A-Z0-9_]+$/.test(t)) role = 'constant';
      else if (/^[A-Z]/.test(t)) role = 'type';
      else if (prev.endsWith('.')) role = 'property';
      else role = 'variable';
    } else if (m[8]) role = (t === '!' && out.length && out[out.length - 1][1] === 'function') ? 'function' : 'operator';
    else if (m[9]) role = 'punctuation';
    out.push([t, role]);
    if (!t.length) re.lastIndex++;
  }
  return out;
}
function tokToml(line) {
  if (/^\s*#/.test(line)) return [[line, 'comment']];
  let m = line.match(/^(\s*)(\[\[?[^\]]+\]\]?)(.*)$/); if (m) return [[m[1], null], [m[2], 'tag'], [m[3], null]];
  m = line.match(/^(\s*)([A-Za-z0-9_\-.]+)(\s*=\s*)(.*)$/);
  if (m) { const v = m[4]; const vt = /^"/.test(v) ? 'string' : /^\d/.test(v) ? 'number' : /^(true|false)/.test(v) ? 'constant' : 'punctuation'; return [[m[1], null], [m[2], 'property'], [m[3], 'operator'], [v, vt]]; }
  return [[line, null]];
}
const tokMd = line => /^#/.test(line) ? [[line, 'keyword']] : [[line, null]];
function lang(path) {
  if (/\.rs$/.test(path)) return { name: 'Rust', tok: tokRust, server: 'rust-analyzer' };
  if (/\.toml$/.test(path)) return { name: 'TOML', tok: tokToml };
  if (/\.md$/.test(path)) return { name: 'Markdown', tok: tokMd };
  return { name: 'Plain text', tok: l => [[l, null]] };
}

// ---------- project tree
function buildTree(paths) {
  const root = { name: '', children: [] };
  for (const p of paths) {
    const parts = p.split('/'); let node = root;
    parts.forEach((part, i) => {
      const dir = i < parts.length - 1;
      let ch = node.children.find(c => c.name === part && !!c.children === dir);
      if (!ch) { ch = dir ? { name: part, children: [] } : { name: part }; node.children.push(ch); }
      node = ch;
    });
  }
  const sort = n => { if (!n.children) return; n.children.sort((a, b) => (!!b.children - !!a.children) || a.name.localeCompare(b.name)); n.children.forEach(sort); };
  sort(root); return root;
}
const TREE = buildTree(C.PATHS);
function treeRows(s) {
  const rows = []; if (s.project === 'empty') return rows;
  const walk = (node, depth, path) => { for (const ch of node.children) { const p = path ? path + '/' + ch.name : ch.name; const open = s.tree.open.has(p);
    rows.push({ name: ch.name, dir: !!ch.children, depth, path: p, open }); if (ch.children && open) walk(ch, depth + 1, p); } };
  walk(TREE, 0, ''); return rows;
}

// ---------- state helpers
const activeTab = (s, pi) => { const p = s.panes[pi == null ? s.fp : pi]; return p && p.tabs[p.active]; };
function buf(s, path) {
  if (!s.bufs[path]) s.bufs[path] = { lines: (C.FILES[path] != null ? C.FILES[path] : path.startsWith('untitled') ? '' : '//! ' + path + '\n').split('\n'), dirty: false };
  return s.bufs[path];
}
function open(s, pi, path, pos) {
  buf(s, path); const p = s.panes[pi]; let i = p.tabs.findIndex(t => t.path === path);
  if (i < 0) { p.tabs.push({ path, l: 0, c: 0, top: 0, left: 0, sel: null }); i = p.tabs.length - 1; }
  p.active = i; if (pos) Object.assign(p.tabs[i], pos);
}
const lineOf = (s, path, sub) => Math.max(0, buf(s, path).lines.findIndex(l => l.includes(sub)));
const digitsOf = n => Math.max(3, String(n).length);
const gutterW = n => digitsOf(n) + 3;

function layout(s) {
  const { W, H } = s; const L = { W, H, status: H - 1 };
  let bottom = H - 1;
  if (s.bar) { L.bar = H - 2; bottom = H - 2; }
  if (s.run && s.run.visible) { const rh = clamp(Math.round(H * .3), 6, 18); L.run = { x: 0, y: bottom - rh, w: W, h: rh }; bottom -= rh; }
  L.mainH = bottom;
  L.treeW = clamp(Math.round(W * .19), 22, 36);
  L.treeShown = s.tree.visible && !(s.split && W < 120);
  let ex = 0;
  if (L.treeShown) { L.tree = { x: 0, y: 0, w: L.treeW, h: bottom }; L.treeDiv = L.treeW; ex = L.treeW + 1; }
  const ew = W - ex;
  if (s.split) { const lw = Math.floor((ew - 1) / 2); L.panes = [{ x: ex, y: 0, w: lw, h: bottom }, { x: ex + lw + 1, y: 0, w: ew - lw - 1, h: bottom }]; L.splitDiv = ex + lw; }
  else L.panes = [{ x: ex, y: 0, w: ew, h: bottom }];
  return L;
}
function follow(s) {
  if (!s.split && s.fp > 0) s.fp = 0;
  const L = layout(s);
  L.panes.forEach((r, pi) => {
    const t = activeTab(s, pi); if (!t) return;
    const lines = buf(s, t.path).lines; t.l = clamp(t.l, 0, lines.length - 1); t.c = clamp(t.c, 0, lines[t.l].length);
    const rows = r.h - 1, tw = r.w - gutterW(lines.length);
    if (t.l < t.top) t.top = t.l; if (t.l >= t.top + rows) t.top = t.l - rows + 1;
    if (t.c < t.left) t.left = Math.max(0, t.c - 4); if (t.c >= t.left + tw) t.left = t.c - tw + 6;
  });
}

// ---------- search
function findAll(lines, q, cs, rx) {
  if (!q) return { list: [] };
  let re;
  try { re = new RegExp(rx ? q : q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), cs ? 'g' : 'gi'); } catch (e) { return { list: [], err: 'invalid regex' }; }
  const list = [];
  lines.forEach((ln, l) => { re.lastIndex = 0; let m; while ((m = re.exec(ln))) { if (!m[0].length) { re.lastIndex++; continue; } list.push({ l, c0: m.index, c1: m.index + m[0].length }); } });
  return { list };
}
function barMatches(s) {
  const b = s.bar; if (!b || b.type !== 'find') return null; const t = activeTab(s); if (!t) return null;
  return findAll(buf(s, t.path).lines, b.q, b.cs, b.rx);
}
function projectHits(s, o) {
  const hits = []; const files = new Set();
  for (const p of C.PATHS) { if (C.FILES[p] == null && !s.bufs[p]) continue;
    const r = findAll(buf(s, p).lines, o.q, o.cs, o.rx); if (r.err) return { hits: [], files: 0, err: r.err };
    r.list.forEach(h => { hits.push(Object.assign({ path: p, text: buf(s, p).lines[h.l] }, h)); files.add(p); }); }
  return { hits, files: files.size };
}
function fuzzy(q, str) {
  if (!q) return { score: 0, idx: [] };
  const s = str.toLowerCase(), ql = q.toLowerCase(), base = str.lastIndexOf('/') + 1;
  const tryAt = from => { const idx = []; let j = 0, score = 0, prev = -2;
    for (let i = from; i < s.length && j < ql.length; i++) if (s[i] === ql[j]) { idx.push(i); score += (i === prev + 1 ? 6 : 1) + (i >= base ? 2 : 0) + (i === 0 || '/_.-'.includes(str[i - 1]) ? 3 : 0); prev = i; j++; }
    return j === ql.length ? { score, idx } : null; };
  const a = tryAt(base); if (a) { a.score += 10 - str.length * .02; return a; }
  const b = tryAt(0); if (b) b.score -= str.length * .02; return b;
}
function pickerItems(s, o) {
  if (o.kind === 'run') return C.RUNS.map(r => ({ label: r.name, r, m: fuzzy(o.q, r.name) })).filter(x => x.m);
  const out = C.PATHS.map(p => ({ label: p, m: fuzzy(o.q, p) })).filter(x => x.m);
  if (o.q) out.sort((a, b) => b.m.score - a.m.score || a.label.length - b.label.length);
  return out;
}
function diagsFor(s, path) {
  if (!s.diag || s.lsp !== 'ready') return [];
  const lines = buf(s, path).lines;
  return C.DIAGS.filter(d => d[0] === path).map(d => { const l = lines.findIndex(x => x.includes(d[1])); if (l < 0) return null; const c0 = lines[l].indexOf(d[2]); return c0 < 0 ? null : { l, c0, c1: c0 + d[2].length, sev: d[3], msg: d[4] }; }).filter(Boolean);
}
const SEV = { err: 'err', warn: 'working', info: 'info' }, SEVG = { err: '\u2715', warn: '\u26a0', info: 'i' };

// ---------- render
function render(s) {
  const g = new Grid(s.W, s.H);
  if (s.W < 100 || s.H < 30) {
    g.region = 'too small';
    const a = 'Glyph needs at least 100\u00d730', b = 'This terminal is ' + s.W + '\u00d7' + s.H;
    const y = Math.floor(s.H / 2) - 1;
    g.put(Math.floor((s.W - len(a)) / 2), y, a, { fg: 'strong', b: 1 });
    g.put(Math.floor((s.W - len(b)) / 2), y + 1, b, { fg: 'muted' });
    return g;
  }
  const L = layout(s); g.L = L;
  if (L.treeShown) renderTree(g, s, L);
  for (let i = 0; i < L.panes.length; i++) renderPane(g, s, L, i);
  g.region = 'divider';
  const div = x => { for (let y = 0; y < L.mainH; y++) g.set(x, y, '\u2502', { fg: 'line', bg: y === 0 ? 'sidebar_bg' : 'bg', b: 0, u: 0 }); };
  if (L.treeShown) div(L.treeDiv);
  if (s.split) div(L.splitDiv);
  if (L.run) renderRun(g, s, L);
  if (s.bar) renderBar(g, s, L);
  renderStatus(g, s, L);
  if (s.popup && !s.overlay && g.cursor) renderPopup(g, s, L);
  if (s.overlay) renderOverlay(g, s, L);
  return g;
}

function renderTree(g, s, L) {
  const r = L.tree, focused = s.focus === 'tree';
  g.region = 'tree';
  g.fill(r.x, r.y, r.w, r.h, { fg: 'text', bg: 'sidebar_bg' });
  g.put(1, 0, s.project === 'empty' ? 'NOTES' : 'GLYPH', { fg: focused ? 'accent' : 'muted', b: 1 });
  const rows = treeRows(s), at = activeTab(s), activePath = at && at.path;
  if (!rows.length) {
    g.put(2, 2, 'No files yet', { fg: 'text' });
    g.put(2, 4, 'a', { fg: 'strong', b: 1 }); g.put(5, 4, 'new file', { fg: 'muted' });
    g.put(2, 5, 'A', { fg: 'strong', b: 1 }); g.put(5, 5, 'new folder', { fg: 'muted' });
    g.hit(r.x, r.y, r.w, r.h, { t: 'treebg' }); return;
  }
  const vis = r.h - 1;
  if (s.tree.sel < s.tree.scroll) s.tree.scroll = s.tree.sel;
  if (s.tree.sel >= s.tree.scroll + vis) s.tree.scroll = s.tree.sel - vis + 1;
  for (let k = 0; k < vis; k++) {
    const i = s.tree.scroll + k, row = rows[i]; if (!row) break; const y = 1 + k;
    const sel = i === s.tree.sel, hov = i === s.tree.hover;
    const bg = sel ? 'hov' : hov ? 'card' : 'sidebar_bg';
    const fg = sel ? (focused ? 'strong' : 'text') : 'text';
    g.fill(r.x, y, r.w, 1, { fg, bg });
    let x = 1 + row.depth * 2;
    x = g.put(x, y, row.dir ? (row.open ? '\u25be ' : '\u25b8 ') : '  ', { fg: 'muted' }, r.w);
    const nameFg = (!row.dir && row.path === activePath && !sel) ? 'accent' : fg;
    g.put(x, y, cut(row.name, r.w - x - 2), { fg: nameFg, b: sel && focused ? 1 : 0 }, r.w - 2);
    if (s.bufs[row.path] && s.bufs[row.path].dirty) g.set(r.w - 2, y, '\u25cf', { fg: 'working' });
    if (sel && focused && !s.overlay && !s.bar) g.cursor = null;
    g.hit(r.x, y, r.w, 1, { t: 'tree', i });
  }
}

function tabLabels(p) {
  const names = p.tabs.map(t => t.path.split('/').pop());
  return p.tabs.map((t, i) => { const n = names[i]; const dup = names.filter(x => x === n).length > 1; const parts = t.path.split('/'); return dup ? n + ' \u00b7 ' + (parts[parts.length - 2] || '') : n; });
}
function renderPane(g, s, L, pi) {
  const r = L.panes[pi], p = s.panes[pi], focused = s.focus === 'pane' && s.fp === pi;
  g.region = 'tab bar';
  g.fill(r.x, r.y, r.w, 1, { fg: 'muted', bg: 'sidebar_bg' });
  const labels = tabLabels(p).map((n, i) => ' ' + n + ' ' + (buf(s, p.tabs[i].path).dirty ? '\u25cf ' : ''));
  let first = 0; const widths = labels.map(len);
  const sum = (a, b) => widths.slice(a, b + 1).reduce((x, y) => x + y, 0);
  const marker = n => n ? ' \u2039' + n + ' ' : '';
  while (first < p.active && sum(first, p.active) + len(marker(first)) > r.w) first++;
  let x = r.x;
  if (first) x = g.put(x, r.y, marker(first), { fg: 'muted' });
  for (let i = first; i < labels.length && x < r.x + r.w; i++) {
    const act = i === p.active;
    const st = act && focused ? { fg: 'tab_active_fg', bg: 'tab_active_bg', b: 1 } : act ? { fg: 'strong', u: 'l', uc: 'strong' } : { fg: 'muted' };
    const x0 = x; x = g.put(x, r.y, labels[i], st, r.x + r.w);
    const dot = labels[i].lastIndexOf('\u25cf');
    if (dot > 0 && !(act && focused)) g.set(x0 + dot, r.y, null, { fg: 'working' });
    g.hit(x0, r.y, x - x0, 1, { t: 'tab', pane: pi, i });
  }
  renderEditor(g, s, r, pi, focused);
}

function renderWelcome(g, s, r) {
  g.region = 'welcome';
  const keys = s.project === 'empty'
    ? [['a', 'new file (tree)'], ['A', 'new folder (tree)'], ['Ctrl+N', 'new untitled buffer'], ['Ctrl+B', 'hide tree']]
    : [['Ctrl+P', 'go to file'], ['Alt+F', 'search the project'], ['Ctrl+N', 'new file'], ['Ctrl+B', 'toggle tree'], ['F5', 'run a command'], ['Ctrl+Q', 'quit']];
  const w = 30, h = keys.length * 2 + 1;
  const x0 = r.x + Math.floor((r.w - w) / 2), y0 = r.y + 1 + Math.max(0, Math.floor((r.h - 1 - h) / 2));
  g.put(x0, y0, 'glyph', { fg: 'strong', b: 1 }); g.put(x0 + 8, y0, s.project === 'empty' ? 'empty project' : 'no file open', { fg: 'muted' });
  keys.forEach(([k, d], i) => { g.put(x0, y0 + 2 + i * 2, k, { fg: 'accent', b: 1 }); g.put(x0 + 10, y0 + 2 + i * 2, d, { fg: 'text' }); });
}

function inRange(l, c, a, b) { return (l > a.l || (l === a.l && c >= a.c)) && (l < b.l || (l === b.l && c < b.c)); }
function renderEditor(g, s, r, pi, focused) {
  const t = activeTab(s, pi);
  g.region = 'editor';
  g.fill(r.x, r.y + 1, r.w, r.h - 1, { fg: 'fg', bg: 'bg' });
  if (!t) { renderWelcome(g, s, r); return; }
  const lines = buf(s, t.path).lines, L = lang(t.path), dg = digitsOf(lines.length), gw = dg + 3, tx = r.x + gw, tw = r.w - gw;
  const diags = diagsFor(s, t.path);
  const fm = focused ? barMatches(s) : null, cur = fm && s.bar.cur;
  let sa = null, sb = null;
  if (t.sel) { const a = t.sel, b = { l: t.l, c: t.c }; [sa, sb] = (a.l < b.l || (a.l === b.l && a.c < b.c)) ? [a, b] : [b, a]; }
  for (let k = 0; k < r.h - 1; k++) {
    const li = t.top + k, y = r.y + 1 + k; if (li >= lines.length) break;
    const isCur = li === t.l, lineBg = isCur && focused ? 'current_line_bg' : 'bg';
    g.region = 'gutter';
    g.fill(r.x, y, gw, 1, { fg: 'gutter_fg', bg: lineBg });
    const dl = diags.filter(d => d.l === li);
    if (dl.length) { const sev = dl.some(d => d.sev === 'err') ? 'err' : dl.some(d => d.sev === 'warn') ? 'warn' : 'info'; g.set(r.x, y, '\u25cf', { fg: SEV[sev] }); }
    g.put(r.x + 1, y, String(li + 1).padStart(dg), isCur ? { fg: focused ? 'gutter_active_fg' : 'text', b: focused ? 1 : 0 } : { fg: 'gutter_fg' });
    g.region = 'editor';
    g.fill(tx, y, tw, 1, { fg: 'fg', bg: lineBg });
    let col = 0; const text = lines[li];
    for (const [tok, role] of L.tok(text)) for (const ch of tok) {
      const vx = col - t.left;
      if (vx >= 0 && vx < tw) {
        const st = { fg: role ? 'syn.' + role : 'fg' };
        const d = dl.find(d => col >= d.c0 && col < d.c1); if (d) { st.u = 'c'; st.uc = SEV[d.sev]; }
        if (sa && inRange(li, col, sa, sb)) st.bg = 'selection_bg';
        if (fm) { const mi = fm.list.findIndex(m => m.l === li && col >= m.c0 && col < m.c1); if (mi >= 0) { if (mi === cur) { st.bg = 'find_current_bg'; st.fg = 'find_current_fg'; } else st.bg = 'find_match_bg'; } }
        g.set(tx + vx, y, ch, st);
      }
      col++;
    }
    if (sa && li >= sa.l && li < sb.l && text.length - t.left >= 0 && text.length - t.left < tw) g.set(tx + text.length - t.left, y, ' ', { bg: 'selection_bg' });
    if (t.left > 0 && text.length > 0) g.set(tx, y, '\u2039', { fg: 'muted', bg: lineBg, u: 0 });
    if (text.length > t.left + tw) g.set(tx + tw - 1, y, '\u203a', { fg: 'muted', bg: lineBg, u: 0 });
  }
  g.hit(r.x, r.y + 1, r.w, r.h - 1, { t: 'editor', pane: pi, x0: tx, y0: r.y + 1 });
  if (focused && !s.overlay) {
    const cx = tx + t.c - t.left, cy = r.y + 1 + t.l - t.top;
    if (cx >= tx && cx < tx + tw && cy > r.y && cy < r.y + r.h) { g.cursorPos = { x: cx, y: cy }; if (!s.bar || s.bar.type === 'find' && false) { const c = g.at(cx, cy); c.cur = 1; } g.cursor = { x: cx, y: cy }; }
  }
}

function field(g, x, y, w, label, text, focused, extra) {
  g.fill(x, y, w, 1, { fg: 'strong', bg: 'card2' });
  let xx = g.put(x + 1, y, label, { fg: focused ? 'text' : 'muted' });
  xx += 2;
  const room = w - (xx - x) - 1;
  const shown = len(text) > room ? '\u2026' + [...text].slice(-(room - 1)).join('') : text;
  const end = g.put(xx, y, shown, Object.assign({ fg: 'strong' }, extra || {}), x + w);
  if (focused) { const c = g.at(Math.min(end, x + w - 1), y); if (c) c.cur = 1; }
  return end;
}
function toggles(g, xr, y, o, count, err) {
  const parts = [[' Aa ', o.cs ? { fg: 'acc_ink', bg: 'accent', b: 1 } : { fg: 'muted' }], [' ', {}], [' .* ', o.rx ? { fg: 'acc_ink', bg: 'accent', b: 1 } : { fg: 'muted' }], ['  ', {}], [count, err ? { fg: 'err' } : { fg: 'strong' }], [' ', {}]];
  let w = parts.reduce((a, p) => a + len(p[0]), 0), x = xr - w;
  for (const [t, st] of parts) x = g.put(x, y, t, st);
  return w;
}
function renderBar(g, s, L) {
  const b = s.bar, y = L.bar, W = s.W;
  g.region = b.type === 'find' ? 'find bar' : 'prompt bar';
  g.fill(0, y, W, 1, { fg: 'strong', bg: 'card2' });
  if (b.type === 'find') {
    const fm = barMatches(s) || { list: [] };
    const count = fm.err ? fm.err : b.q ? (fm.list.length ? (b.cur + 1) + '/' + fm.list.length : '0/0') : '';
    const tw = toggles(g, W, y, b, count, fm.err);
    if (b.r == null) field(g, 0, y, W - tw, 'Find', b.q, true, fm.err ? { fg: 'err' } : null);
    else {
      const half = Math.floor(W / 2);
      field(g, 0, y, half, 'Find', b.q, !b.inR, fm.err ? { fg: 'err' } : null);
      g.set(half, y, '\u2502', { fg: 'line', bg: 'card2' });
      field(g, half + 1, y, W - half - 1 - tw, 'Replace', b.r, b.inR);
    }
  } else {
    const hint = b.hint || '\u23ce ok   esc cancel';
    field(g, 0, y, W - len(hint) - 2, b.label, b.text, true, null);
    if (b.selTo) for (let i = 0; i < b.selTo; i++) { const c = g.at(len(b.label) + 3 + i, y); c.bg = 'selection_bg'; }
    g.put(W - len(hint) - 1, y, hint, { fg: 'muted' });
  }
}

function lspSeg(s, L) {
  if (!L || !L.server) return null;
  const n = L.server;
  return { ready: [['\u25cf ', 'done'], [n, 'text']], starting: [['\u25cb ', 'working'], [n, 'text']], notfound: [['\u25cb ', 'muted'], ['no server', 'muted']], crashed: [['\u2715 ', 'err'], [n, 'err']] }[s.lsp];
}
function renderStatus(g, s, L) {
  const y = L.status, W = s.W; g.region = 'status';
  g.fill(0, y, W, 1, { fg: 'text', bg: 'sidebar_bg' });
  const t = activeTab(s);
  let msg = null;
  if (s.message) msg = [[s.message.icon ? s.message.icon + ' ' : '', s.message.role || 'text'], [s.message.t, s.message.role === 'err' ? 'err' : 'text']];
  else if (t) { const d = diagsFor(s, t.path).find(d => d.l === t.l && t.c >= d.c0 && t.c <= d.c1); if (d) msg = [[SEVG[d.sev] + ' ', SEV[d.sev]], [d.msg, 'text']]; }
  const segs = [];
  if (t) {
    const lg = lang(t.path), dirty = buf(s, t.path).dirty, parts = t.path.split('/'), base = parts.pop(), dir = parts.length ? parts.join('/') + '/' : '';
    segs.push({ k: 'path', v: [[dir, 'muted'], [base, 'strong']].concat(dirty ? [[' \u25cf', 'working']] : []), short: [[base, 'strong']].concat(dirty ? [[' \u25cf', 'working']] : []) });
    segs.push({ k: 'pos', v: [['Ln ' + (t.l + 1) + ', Col ' + (t.c + 1), 'text']] });
    segs.push({ k: 'lang', v: [[lg.name, 'text']] });
    const ls = lspSeg(s, lg); if (ls) segs.push({ k: 'lsp', v: ls });
    const ds = diagsFor(s, t.path), w = ds.filter(d => d.sev === 'warn').length, e = ds.filter(d => d.sev === 'err').length;
    if (w + e) segs.push({ k: 'counts', v: [['\u26a0 ' + w, 'working'], ['  ', 'text'], ['\u2715 ' + e, 'err']] });
  } else segs.push({ k: 'path', v: [[s.project === 'empty' ? 'C:\\src\\notes' : 'C:\\src\\glyph', 'muted']] });
  const wOf = arr => arr.reduce((a, p) => a + len(p[0]), 0);
  const total = () => segs.reduce((a, sg) => a + wOf(sg.v) + 2, 0) + 1;
  const minMsg = msg ? Math.min(wOf(msg), 24) : 0;
  for (const k of ['lsp', 'lang']) if (total() + minMsg + 2 > W) { const i = segs.findIndex(x => x.k === k); if (i >= 0) segs.splice(i, 1); }
  if (total() + minMsg + 2 > W) { const p = segs.find(x => x.k === 'path'); if (p && p.short) p.v = p.short; }
  let x = W - total() + 2;
  const right0 = x;
  segs.forEach((sg, i) => { for (const [txt, role] of sg.v) x = g.put(x, y, txt, { fg: role }); x += 2; });
  if (msg) {
    let mx = 1; const room = right0 - 3;
    for (const [txt, role] of msg) { const left = room - (mx - 1); if (left <= 0) break; mx = g.put(mx, y, cut(txt, left), { fg: role }); }
  }
}

function renderRun(g, s, L) {
  const r = L.run, run = s.run, focused = s.focus === 'run'; g.region = 'run panel';
  g.fill(r.x, r.y, r.w, r.h, { fg: 'fg', bg: 'bg' });
  g.fill(r.x, r.y, r.w, 1, { fg: 'text', bg: 'sidebar_bg' });
  g.hit(r.x, r.y, r.w, r.h, { t: 'run' });
  if (!run.name) {
    g.put(1, r.y, 'run', { fg: focused ? 'accent' : 'strong', b: 1 });
    g.put(2, r.y + 1, 'F5 runs a command from .griffin.toml', { fg: 'muted' });
    g.put(r.w - 9, r.y, 'F4 hide', { fg: 'muted' });
    return;
  }
  const st = { running: ['\u25cf', 'working', 'running'], stopped: ['\u25a0', 'idle', 'stopped'], exited: run.code === 0 ? ['\u2713', 'done', 'exited 0'] : ['\u2715', 'err', 'exited ' + run.code] }[run.status];
  let x = g.put(1, r.y, st[0] + ' ', { fg: st[1] });
  x = g.put(x, r.y, run.name, { fg: focused ? 'accent' : 'strong', b: 1 });
  x = g.put(x + 2, r.y, st[2], { fg: st[1] });
  const hints = run.status === 'running' ? 'shift+F5 stop   ctrl+F5 restart   F4 hide' : 'F5 run again   F4 hide';
  g.put(x + 3, r.y, cut(run.cmd, Math.max(0, r.w - x - len(hints) - 8)), { fg: 'muted' });
  g.put(r.w - len(hints) - 1, r.y, hints, { fg: 'muted' });
  const rows = r.h - 1, lines = run.lines.slice(-rows);
  lines.forEach((ln, k) => {
    let xx = 1; const y = r.y + 1 + k;
    if (ln.marker) { g.put(1, y, '\u2500\u2500 restarted ' + ln.marker + ' ' + '\u2500'.repeat(Math.max(0, r.w - 26)), { fg: 'muted', d: 1 }); return; }
    for (const [txt, role, bold] of ln) xx = g.put(xx, y, txt, { fg: role || 'fg', b: bold ? 1 : 0 }, r.w - 1);
  });
}

function anchor(cur, w, h, b) {
  w = Math.min(w, b.w); const x = Math.max(b.x, Math.min(cur.x, b.x + b.w - w));
  const below = b.y + b.h - (cur.y + 1), above = cur.y - b.y;
  if (h <= below) return { x, y: cur.y + 1, w, h, flip: false };
  if (h <= above) return { x, y: cur.y - h, w, h, flip: true };
  return below >= above ? { x, y: cur.y + 1, w, h: below } : { x, y: b.y, w, h: above, flip: true };
}
function boxCard(g, r) {
  g.fill(r.x, r.y, r.w, r.h, { fg: 'text', bg: 'card' });
  const st = { fg: 'border', bg: 'card' };
  for (let x = r.x + 1; x < r.x + r.w - 1; x++) { g.set(x, r.y, '\u2500', st); g.set(x, r.y + r.h - 1, '\u2500', st); }
  for (let y = r.y + 1; y < r.y + r.h - 1; y++) { g.set(r.x, y, '\u2502', st); g.set(r.x + r.w - 1, y, '\u2502', st); }
  g.set(r.x, r.y, '\u250c', st); g.set(r.x + r.w - 1, r.y, '\u2510', st); g.set(r.x, r.y + r.h - 1, '\u2514', st); g.set(r.x + r.w - 1, r.y + r.h - 1, '\u2518', st);
}
function wrap(text, w) { const out = []; let cur = ''; for (const word of text.split(' ')) { if (len(cur) + len(word) + (cur ? 1 : 0) > w && cur) { out.push(cur); cur = word; } else cur = cur ? cur + ' ' + word : word; } if (cur) out.push(cur); return out; }
function compItems(s) {
  const p = s.popup, t = activeTab(s), typed = buf(s, t.path).lines[t.l].slice(p.start, t.c);
  return { typed, items: C.COMPLETIONS.filter(c => c[1].toLowerCase().startsWith(typed.toLowerCase())).slice(0, 10) };
}
function renderPopup(g, s, L) {
  const p = s.popup, cur = g.cursor, bounds = { x: 0, y: 0, w: s.W, h: L.bar != null ? L.bar : L.status };
  if (p.type === 'hover') {
    g.region = 'hover popup';
    const tw = Math.min(60, bounds.w - 4), doc = wrap(C.HOVER.doc, tw);
    const lines = C.HOVER.code.length + 1 + doc.length;
    const w = Math.min(tw, Math.max(...C.HOVER.code.map(len), ...doc.map(len))) + 4;
    const r = anchor(cur, w, lines + 2, bounds); boxCard(g, r);
    C.HOVER.code.forEach((ln, i) => { let x = r.x + 2; for (const [tok, role] of tokRust(ln)) x = g.put(x, r.y + 1 + i, tok, { fg: role ? 'syn.' + role : 'fg' }, r.x + r.w - 2); });
    const sy = r.y + 1 + C.HOVER.code.length;
    for (let x = r.x + 1; x < r.x + r.w - 1; x++) g.set(x, sy, '\u2500', { fg: 'border' });
    g.set(r.x, sy, '\u251c', { fg: 'border' }); g.set(r.x + r.w - 1, sy, '\u2524', { fg: 'border' });
    doc.forEach((ln, i) => { if (sy + 1 + i < r.y + r.h - 1) g.put(r.x + 2, sy + 1 + i, ln, { fg: 'text' }); });
  } else {
    g.region = 'completion popup';
    const { typed, items } = compItems(s); if (!items.length) return;
    const kw = Math.max(...items.map(i => len(i[0]))), lw = Math.max(...items.map(i => len(i[1]))), dw = Math.max(...items.map(i => len(i[2])));
    const w = kw + 1 + lw + 3 + dw + 4;
    const at = { x: cur.x - len(typed) - kw - 3, y: cur.y };
    const r = anchor(at, w, items.length + 2, bounds); boxCard(g, r);
    const sel = Math.min(p.sel, items.length - 1);
    items.forEach((it, i) => {
      const y = r.y + 1 + i; if (y >= r.y + r.h - 1) return; const on = i === sel;
      g.fill(r.x + 1, y, r.w - 2, 1, { fg: on ? 'strong' : 'fg', bg: on ? 'hov' : 'card' });
      g.put(r.x + 2, y, it[0], { fg: 'muted' });
      const lx = r.x + 2 + kw + 1;
      g.put(lx, y, it[1].slice(0, typed.length), { fg: 'accent', b: 1 });
      g.put(lx + typed.length, y, it[1].slice(typed.length), { fg: on ? 'strong' : 'fg' });
      g.put(r.x + r.w - 2 - len(it[2]), y, it[2], { fg: 'muted' });
    });
  }
}

// ---------- overlays (dialogs)
function dimAll(g) { for (const c of g.cells) { c.dim = 1; c.cur = 0; } }
function dialog(g, s, w, h, name) {
  g.region = name; w = Math.min(w, s.W - 2); h = Math.min(h, s.H - 2);
  const r = { x: Math.floor((s.W - w) / 2), y: Math.floor((s.H - h) / 2), w, h };
  g.fill(r.x, r.y, r.w, r.h, { fg: 'text', bg: 'card' });
  g.hit(0, 0, s.W, s.H, { t: 'scrim' }); g.hit(r.x, r.y, r.w, r.h, { t: 'card' });
  return r;
}
function footer(g, r, left, right) { const y = r.y + r.h - 1; g.put(r.x + 2, y, left, { fg: 'muted' }, r.x + r.w - 2); if (right) g.put(r.x + r.w - 2 - len(right), y, right, { fg: 'muted' }); }
const CONFIRMS = {
  unsaved: o => ({ q: o.file + ' has unsaved changes', sub: 'Closing discards them unless you save.', btns: [['s', 'Save'], ['d', 'Discard'], ['c', 'Cancel']] }),
  recover: o => ({ q: 'Recover unsaved changes to ' + o.file + '?', sub: 'A backup from 14:02 is newer than the file on disk.', btns: [['r', 'Recover'], ['d', 'Discard']] }),
  replace: o => ({ q: 'Replace ' + o.n + ' matches in ' + o.m + ' files?', sub: 'Open buffers are edited in place (undo with Ctrl+Z); other files are saved.', btns: [['r', 'Replace'], ['c', 'Cancel']] }),
  trash: o => ({ q: 'Move ' + o.file + ' to the trash?', sub: 'You can restore it from the system trash.', btns: [['y', 'Move to trash'], ['n', 'Cancel']] }),
};
function renderOverlay(g, s, L) {
  const o = s.overlay; dimAll(g);
  if (o.type === 'picker') {
    const items = pickerItems(s, o), run = o.kind === 'run';
    const w = clamp(Math.floor(s.W * 3 / 5), 40, 80), h = Math.min(run ? 8 : 20, s.H - 4);
    const r = dialog(g, s, w, h, run ? 'run picker' : 'go to file');
    field(g, r.x, r.y, r.w, run ? 'Run' : 'Go to file', o.q, true);
    const rows = r.h - 2; o.sel = clamp(o.sel, 0, Math.max(0, items.length - 1));
    const first = Math.max(0, o.sel - (rows - 1));
    if (!items.length) g.put(r.x + 2, r.y + 1, run ? 'no match' : 'no matching files', { fg: 'muted' });
    items.slice(first, first + rows).forEach((it, k) => {
      const i = first + k, y = r.y + 1 + k, on = i === o.sel;
      g.fill(r.x, y, r.w, 1, { fg: 'text', bg: on ? 'hov' : 'card' });
      const base = it.label.lastIndexOf('/') + 1;
      [...it.label].forEach((ch, ci) => {
        const hit = it.m.idx.includes(ci);
        const st = hit ? (on ? { fg: 'acc_ink', bg: 'accent', b: 1 } : { fg: 'accent', b: 1 }) : { fg: ci < base ? (on ? 'text' : 'muted') : 'strong' };
        if (r.x + 2 + ci < r.x + r.w - 2) g.set(r.x + 2 + ci, y, ch, st);
      });
      if (run) { g.put(r.x + 4 + 10, y, cut(it.r.cmd, r.w - 34), { fg: 'muted' }); g.put(r.x + r.w - 2 - len(it.r.src), y, it.r.src, { fg: 'muted' }); }
      g.hit(r.x, y, r.w, 1, { t: 'pick', i });
    });
    footer(g, r, '\u2191\u2193 select   \u23ce ' + (run ? 'run' : 'open') + '   esc close', run ? '' : items.length + ' of ' + C.PATHS.length);
  } else if (o.type === 'goto') {
    const t = activeTab(s), n = buf(s, t.path).lines.length, v = parseInt(o.text, 10), bad = o.text && !(v >= 1 && v <= n);
    const r = dialog(g, s, 44, 2, 'go to line');
    field(g, r.x, r.y, r.w, 'Go to line', o.text, true, bad ? { fg: 'err' } : null);
    footer(g, r, '\u23ce go   esc cancel', bad ? '' : '1\u2013' + n);
    if (bad) g.put(r.x + r.w - 2 - len('1\u2013' + n), r.y + 1, '1\u2013' + n, { fg: 'err' });
  } else if (o.type === 'search') {
    const w = clamp(s.W - 20, 60, 140), h = clamp(s.H - 8, 14, 40);
    const r = dialog(g, s, w, h, 'project search');
    const res = projectHits(s, o);
    const tw = toggles(g, r.x + r.w, r.y, o, '', false);
    g.fill(r.x, r.y, r.w, 2, { fg: 'strong', bg: 'card2' });
    field(g, r.x, r.y, r.w - tw, 'Search ', o.q, !o.inR, res.err ? { fg: 'err' } : null);
    toggles(g, r.x + r.w, r.y, o, '', false);
    field(g, r.x, r.y + 1, r.w, 'Replace', o.r || '', o.inR);
    if (!o.inR && !o.r) g.put(r.x + 11, r.y + 1, 'tab to replace', { fg: 'muted' });
    g.put(r.x + 2, r.y + 2, res.err ? res.err : o.q ? res.hits.length + ' matches in ' + res.files + ' files' : 'type to search the project (.gitignore respected)', { fg: res.err ? 'err' : 'muted' });
    const rows = r.h - 4; o.sel = clamp(o.sel, 0, Math.max(0, res.hits.length - 1));
    const first = Math.max(0, o.sel - (rows - 1));
    const pw = Math.min(34, Math.max(0, ...res.hits.map(h => len(h.path + ':' + (h.l + 1)))));
    res.hits.slice(first, first + rows).forEach((h, k) => {
      const i = first + k, y = r.y + 3 + k, on = i === o.sel;
      g.fill(r.x, y, r.w, 1, { fg: 'text', bg: on ? 'hov' : 'card' });
      const parts = h.path.split('/'), base = parts.pop(), dir = parts.length ? parts.join('/') + '/' : '';
      let x = g.put(r.x + 2, y, dir, { fg: on ? 'text' : 'muted' }); x = g.put(x, y, base, { fg: 'strong' }); g.put(x, y, ':' + (h.l + 1), { fg: 'muted' });
      const tx0 = r.x + 2 + pw + 2, room = r.x + r.w - 2 - tx0;
      const ind = h.text.length - h.text.trimStart().length; let start = ind;
      if (h.c0 - start > room - 20) start = h.c0 - 20;
      let xx = tx0; if (start > ind) xx = g.put(xx, y, '\u2026', { fg: 'muted' });
      for (let c = start; c < h.text.length && xx < tx0 + room; c++) {
        const inM = c >= h.c0 && c < h.c1;
        if (inM && o.r) { xx = g.put(xx, y, h.text[c], { fg: 'err', u: 'l', uc: 'err' }); if (c === h.c1 - 1) xx = g.put(xx, y, o.r, { fg: 'done', b: 1 }, tx0 + room); }
        else xx = g.put(xx, y, h.text[c], inM ? (on ? { fg: 'find_current_fg', bg: 'find_current_bg' } : { fg: 'fg', bg: 'find_match_bg' }) : { fg: 'fg' });
      }
      g.hit(r.x, y, r.w, 1, { t: 'shit', i });
    });
    footer(g, r, '\u2191\u2193 select   \u23ce open   tab ' + (o.inR ? 'search' : 'replace') + ' field   alt+a replace all   esc close');
  } else if (o.type === 'confirm') {
    const c = CONFIRMS[o.kind](o); const w = Math.max(len(c.q), len(c.sub)) + 8, h = 7;
    const r = dialog(g, s, Math.max(w, 44), h, 'confirm: ' + o.kind);
    g.put(r.x + 3, r.y + 1, c.q, { fg: 'strong', b: 1 });
    g.put(r.x + 3, r.y + 2, cut(c.sub, r.w - 6), { fg: 'muted' });
    let x = r.x + 3; const y = r.y + 4;
    c.btns.forEach(([k, label], i) => {
      const on = i === (o.btn || 0), txt = ' ' + label + ' ';
      const st = on ? { fg: 'tab_active_fg', bg: 'tab_active_bg', b: 1 } : { fg: 'strong', bg: 'btn' };
      const ki = label.toLowerCase().indexOf(k);
      g.put(x, y, txt, st); g.set(x + 1 + ki, y, null, { u: 'l', uc: on ? 'tab_active_fg' : 'accent' });
      g.hit(x, y, len(txt), 1, { t: 'btn', key: k });
      x += len(txt) + 2;
    });
    g.put(r.x + r.w - 3 - len('esc'), y, 'esc', { fg: 'muted' });
  }
}

// ---------- HTML
const esc = c => c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : c;
function toHTML(g, th, o) {
  const cw = o.cw, lh = o.lh, r = th.r, out = [];
  const res = k => !k ? null : k[0] === '#' ? k : (r[k] || '#ff00ff');
  for (let y = 0; y < g.H; y++) {
    let row = '', runKey = null, runTxt = '', runCss = '';
    const flush = () => { if (runTxt) row += '<span style="' + runCss + '">' + runTxt + '</span>'; runTxt = ''; };
    for (let x = 0; x < g.W; x++) {
      const c = g.cells[y * g.W + x];
      let fg = res(c.fg), bg = res(c.bg), uc = res(c.uc), m = th.mods[c.fg] || {}, b = c.b || m.b, d = c.d || m.d;
      if (c.dim) { if (th.mono) d = 1; else { fg = mix(fg, r.scrim, .6); bg = mix(bg, r.scrim, .6); if (uc) uc = mix(uc, r.scrim, .6); } }
      if (d) fg = mix(fg, bg, .45);
      let css = 'color:' + fg + ';background:' + bg + (b ? ';font-weight:700' : '') + (c.i ? ';font-style:italic' : '');
      if (c.u) css += ';text-decoration:underline ' + (c.u === 'c' ? 'wavy ' : '') + (uc || fg) + ';text-underline-offset:3px;text-decoration-thickness:1px';
      if (c.cur && o.cursor !== false) css += ';box-shadow:inset 2px 0 0 ' + (th.r.strong);
      const wide = c.c.charCodeAt(0) > 127;
      const ch = wide ? '<span style="display:inline-block;width:' + cw + 'px;height:' + lh + 'px;vertical-align:top;text-align:center;overflow:hidden">' + esc(c.c) + (c.c === '\u26a0' ? '\ufe0e' : '') + '</span>' : esc(c.c);
      if (css !== runCss) { flush(); runCss = css; }
      runTxt += ch;
    }
    flush();
    out.push('<div style="height:' + lh + 'px;line-height:' + lh + 'px;white-space:pre">' + row + '</div>');
  }
  return out.join('');
}

// ---------- input
function keyName(e) {
  let k = e.key;
  if (e.altKey && e.code) { if (/^Key[A-Z]$/.test(e.code)) k = e.code.slice(3).toLowerCase(); else if (/^Digit\d$/.test(e.code)) k = e.code.slice(5); else k = ({ Comma: ',', Period: '.', Slash: '/' })[e.code] || k; }
  const map = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', Escape: 'esc', Enter: 'enter', Tab: 'tab', Backspace: 'backspace', Delete: 'delete', Home: 'home', End: 'end', PageUp: 'pgup', PageDown: 'pgdn', ' ': 'space' };
  k = map[k] || (k.length === 1 ? k : k.toLowerCase());
  const shiftable = k.length > 1 || (e.ctrlKey || e.altKey);
  return (e.ctrlKey ? 'ctrl+' : '') + (e.altKey ? 'alt+' : '') + (e.shiftKey && shiftable && k.length > 1 ? 'shift+' : '') + (e.ctrlKey && k.length === 1 ? k.toLowerCase() : k);
}
const isText = k => k.length === 1;
function editField(o, prop, k) { if (isText(k)) { o[prop] = (o[prop] || '') + k; return true; } if (k === 'backspace') { o[prop] = (o[prop] || '').slice(0, -1); return true; } return false; }
function setMsg(s, t, role, icon) { s.message = { t, role, icon }; }

function confirmAnswer(s, key) {
  const o = s.overlay; s.overlay = null;
  if (key === 'c' || key === 'n' || key === 'esc') return;
  if (o.kind === 'unsaved') { const b = s.bufs[o.path]; if (key === 's') { b.dirty = false; setMsg(s, 'Saved ' + o.file, 'done', '\u2713'); } else setMsg(s, 'Discarded changes to ' + o.file); if (o.then) o.then(); }
  else if (o.kind === 'recover') setMsg(s, key === 'r' ? 'Recovered ' + o.file + ' from backup' : 'Backup discarded', key === 'r' ? 'done' : 'text', key === 'r' ? '\u2713' : '');
  else if (o.kind === 'replace') {
    const src = o.src; let n = 0; const files = new Set();
    for (const h of projectHits(s, src).hits) files.add(h.path);
    files.forEach(p => { const b = buf(s, p); let re; try { re = new RegExp(src.rx ? src.q : src.q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), src.cs ? 'g' : 'gi'); } catch (e) { return; } b.lines = b.lines.map(l => l.replace(re, () => { n++; return src.r; })); b.dirty = true; });
    setMsg(s, 'Replaced ' + n + ' matches in ' + files.size + ' files', 'done', '\u2713');
  } else if (o.kind === 'trash') setMsg(s, 'Moved ' + o.file + ' to the trash');
}
function closeTab(s) {
  const p = s.panes[s.fp], t = activeTab(s); if (!t) return;
  const doClose = () => { p.tabs.splice(p.active, 1); p.active = Math.max(0, Math.min(p.active, p.tabs.length - 1)); };
  if (buf(s, t.path).dirty) s.overlay = { type: 'confirm', kind: 'unsaved', file: t.path.split('/').pop(), path: t.path, then: doClose };
  else doClose();
}
function startRun(s, r, restart) {
  const feed = r.name === 'test' ? C.FEED.testFail : r.name === 'cargo run' ? C.FEED.testOk : C.FEED.dev;
  const lines = restart && s.run && s.run.lines ? s.run.lines.concat([{ marker: '14:0' + (3 + (s.clock % 6)) + ':22' }]) : [];
  s.run = { visible: true, name: r.name, cmd: r.cmd, status: 'running', code: null, lines, feed: feed.slice(), loop: r.name === 'dev', n: 0 };
}
function tick(s) {
  const run = s.run; s.clock++;
  if (!run || run.status !== 'running') return false;
  if (run.feed.length) { run.lines.push(run.feed.shift()); if (!run.feed.length && !run.loop) { run.status = 'exited'; run.code = run.name === 'test' ? 101 : 0; } return true; }
  if (run.loop) { const l = C.FEED.devLoop[run.n++ % C.FEED.devLoop.length]; const sec = String(11 + run.n * 3 % 49).padStart(2, '0'); run.lines.push([['14:02:' + sec + ' ', 'muted'], [l[0].padEnd(6), l[1]], [l[2]]]); return true; }
  return false;
}
function treeActivate(s, focusEditor) {
  const row = treeRows(s)[s.tree.sel]; if (!row) return;
  if (row.dir) { row.open ? s.tree.open.delete(row.path) : s.tree.open.add(row.path); }
  else { open(s, s.fp, row.path); if (focusEditor) s.focus = 'pane'; }
}
function selectTreePath(s, path) { const i = treeRows(s).findIndex(r => r.path === path); if (i >= 0) s.tree.sel = i; }
function moveCursor(s, t, k) {
  const lines = buf(s, t.path).lines, shift = k.startsWith('shift+'), d = k.replace('shift+', '').replace('ctrl+', '');
  if (shift) { if (!t.sel) t.sel = { l: t.l, c: t.c }; } else t.sel = null;
  if (d === 'up') t.l = Math.max(0, t.l - 1); else if (d === 'down') t.l = Math.min(lines.length - 1, t.l + 1);
  else if (d === 'left') { if (t.c > 0) t.c--; else if (t.l > 0) { t.l--; t.c = lines[t.l].length; } }
  else if (d === 'right') { if (t.c < lines[t.l].length) t.c++; else if (t.l < lines.length - 1) { t.l++; t.c = 0; } }
  else if (d === 'home') t.c = k.includes('ctrl') ? (t.l = 0, 0) : 0; else if (d === 'end') { if (k.includes('ctrl')) t.l = lines.length - 1; t.c = lines[t.l].length; }
  else if (d === 'pgup') t.l = Math.max(0, t.l - 20); else if (d === 'pgdn') t.l = Math.min(lines.length - 1, t.l + 20);
  t.c = Math.min(t.c, lines[t.l].length);
  if (t.sel && t.sel.l === t.l && t.sel.c === t.c) t.sel = null;
}
function syncFind(s) { const b = s.bar, t = activeTab(s); const fm = barMatches(s); if (!fm || !t) return; const i = fm.list.findIndex(m => m.l > t.l || (m.l === t.l && m.c0 >= (b.origin != null ? b.origin : t.c))); b.cur = fm.list.length ? (i < 0 ? 0 : i) : 0; jumpFind(s); }
function jumpFind(s) { const fm = barMatches(s), t = activeTab(s); const m = fm && fm.list[s.bar.cur]; if (m) { t.l = m.l; t.c = m.c0; t.sel = null; } }

function key(s, e) {
  const k = keyName(e);
  if (s.W < 100 || s.H < 30) return false;
  if (k === 'ctrl+q') { const d = Object.keys(s.bufs).find(p => s.bufs[p].dirty); if (d) { s.overlay = { type: 'confirm', kind: 'unsaved', file: d.split('/').pop(), path: d, then: () => setMsg(s, 'Quit (prototype keeps running)') }; } else setMsg(s, 'Quit (prototype keeps running)'); return true; }
  const o = s.overlay;
  if (o) {
    if (k === 'esc') { s.overlay = null; return true; }
    if (o.type === 'confirm') {
      const c = CONFIRMS[o.kind](o);
      if (k === 'left' || k === 'shift+tab') o.btn = Math.max(0, (o.btn || 0) - 1);
      else if (k === 'right' || k === 'tab') o.btn = Math.min(c.btns.length - 1, (o.btn || 0) + 1);
      else if (k === 'enter') confirmAnswer(s, c.btns[o.btn || 0][0]);
      else { const b = c.btns.find(b => b[0] === k.toLowerCase()); if (b) confirmAnswer(s, b[0]); }
      return true;
    }
    if (o.type === 'picker') {
      const items = pickerItems(s, o);
      if (k === 'up') o.sel = Math.max(0, o.sel - 1); else if (k === 'down') o.sel = Math.min(items.length - 1, o.sel + 1);
      else if (k === 'enter') { const it = items[o.sel]; if (it) { s.overlay = null; if (o.kind === 'run') startRun(s, it.r); else { open(s, s.fp, it.label); s.focus = 'pane'; selectTreePath(s, it.label); } } }
      else if (editField(o, 'q', k)) o.sel = 0;
      follow(s); return true;
    }
    if (o.type === 'goto') {
      if (k === 'enter') { const t = activeTab(s), v = parseInt(o.text, 10), n = buf(s, t.path).lines.length; if (v >= 1 && v <= n) { t.l = v - 1; t.c = 0; t.sel = null; s.overlay = null; follow(s); } }
      else if (/^\d$/.test(k) || k === 'backspace') editField(o, 'text', k);
      return true;
    }
    if (o.type === 'search') {
      const res = projectHits(s, o);
      if (k === 'tab' || k === 'shift+tab') o.inR = !o.inR;
      else if (k === 'up') o.sel = Math.max(0, o.sel - 1); else if (k === 'down') o.sel = Math.min(res.hits.length - 1, o.sel + 1);
      else if (k === 'alt+c') o.cs = !o.cs; else if (k === 'alt+r') o.rx = !o.rx;
      else if (k === 'alt+a') { if (res.hits.length) s.overlay = { type: 'confirm', kind: 'replace', n: res.hits.length, m: res.files, src: o }; }
      else if (k === 'enter') { const h = res.hits[o.sel]; if (h) { s.overlay = null; open(s, s.fp, h.path, { l: h.l, c: h.c0, sel: null }); s.focus = 'pane'; follow(s); } }
      else if (editField(o, o.inR ? 'r' : 'q', k)) { if (!o.inR) o.sel = 0; }
      return true;
    }
  }
  if (s.popup) {
    const p = s.popup;
    if (p.type === 'hover') { s.popup = null; if (k === 'esc') return true; }
    else {
      const t = activeTab(s), { items } = compItems(s);
      if (k === 'esc') { s.popup = null; return true; }
      if (k === 'up') { p.sel = Math.max(0, p.sel - 1); return true; } if (k === 'down') { p.sel = Math.min(items.length - 1, p.sel + 1); return true; }
      if (k === 'enter' || k === 'tab') { const it = items[Math.min(p.sel, items.length - 1)]; if (it) { const b = buf(s, t.path), ln = b.lines[t.l]; b.lines[t.l] = ln.slice(0, p.start) + it[1] + ln.slice(t.c); t.c = p.start + it[1].length; b.dirty = true; } s.popup = null; return true; }
      if (!(isText(k) && /\w/.test(k)) && k !== 'backspace') s.popup = null;
    }
  }
  if (s.bar) {
    const b = s.bar;
    if (k === 'esc') { s.bar = null; return true; }
    if (b.type === 'find') {
      const fm = barMatches(s);
      if (k === 'alt+c') { b.cs = !b.cs; syncFind(s); } else if (k === 'alt+r') { b.rx = !b.rx; syncFind(s); }
      else if (k === 'tab' && b.r != null) b.inR = !b.inR;
      else if (k === 'ctrl+r') { if (b.r == null) b.r = ''; b.inR = true; }
      else if ((k === 'enter' && !b.inR) || k === 'shift+enter') { if (fm.list.length) { b.cur = (b.cur + (k === 'enter' ? 1 : fm.list.length - 1)) % fm.list.length; jumpFind(s); } }
      else if (k === 'enter' && b.inR) { const m = fm.list[b.cur]; if (m) { const t = activeTab(s), bf = buf(s, t.path); bf.lines[m.l] = bf.lines[m.l].slice(0, m.c0) + b.r + bf.lines[m.l].slice(m.c1); bf.dirty = true; syncFind(s); } }
      else if (k === 'alt+a' && b.r != null) { const t = activeTab(s), bf = buf(s, t.path); const n = fm.list.length; for (const m of fm.list.slice().reverse()) bf.lines[m.l] = bf.lines[m.l].slice(0, m.c0) + b.r + bf.lines[m.l].slice(m.c1); if (n) { bf.dirty = true; setMsg(s, 'Replaced ' + n + ' matches', 'done', '\u2713'); } s.bar = null; }
      else if (editField(b, b.inR ? 'r' : 'q', k)) { if (!b.inR) syncFind(s); }
      else return false;
      follow(s); return true;
    }
    if (k === 'enter') { setMsg(s, (b.label === 'Rename' ? 'Renamed to ' : 'Created ') + b.text, 'done', '\u2713'); s.bar = null; return true; }
    editField(b, 'text', k); b.selTo = 0; return true;
  }
  // global
  const t = activeTab(s);
  switch (k) {
    case 'ctrl+p': s.overlay = { type: 'picker', kind: 'file', q: '', sel: 0 }; return true;
    case 'ctrl+g': if (t) s.overlay = { type: 'goto', text: '' }; return true;
    case 'alt+f': s.overlay = { type: 'search', q: '', r: '', inR: false, cs: false, rx: false, sel: 0 }; return true;
    case 'ctrl+f': case 'ctrl+r': if (t) { s.bar = { type: 'find', q: '', r: k === 'ctrl+r' ? '' : null, inR: false, cs: false, rx: false, cur: 0, origin: t.c }; } return true;
    case 'ctrl+b': s.tree.visible = !s.tree.visible; if (!s.tree.visible && s.focus === 'tree') s.focus = 'pane'; follow(s); return true;
    case 'ctrl+e': if (s.tree.visible) s.focus = s.focus === 'tree' ? 'pane' : 'tree'; return true;
    case 'alt+v': s.split = !s.split; if (s.split && !s.panes[1]) { s.panes[1] = { tabs: [], active: 0 }; if (t) open(s, 1, t.path, { l: t.l, c: t.c, top: t.top }); } if (!s.split) s.fp = 0; follow(s); return true;
    case 'f6': { const order = []; if (layout(s).treeShown) order.push('tree'); order.push('p0'); if (s.split) order.push('p1'); if (s.run && s.run.visible) order.push('run'); const curK = s.focus === 'pane' ? 'p' + s.fp : s.focus; const nx = order[(order.indexOf(curK) + 1) % order.length]; if (nx[0] === 'p') { s.focus = 'pane'; s.fp = +nx[1]; } else s.focus = nx; return true; }
    case 'f4': if (!s.run) s.run = { visible: true }; else s.run.visible = !s.run.visible; if (!s.run.visible && s.focus === 'run') s.focus = 'pane'; follow(s); return true;
    case 'f5': s.overlay = { type: 'picker', kind: 'run', q: '', sel: 0 }; return true;
    case 'shift+f5': if (s.run && s.run.status === 'running') { s.run.status = 'stopped'; } return true;
    case 'ctrl+f5': if (s.run && s.run.name) startRun(s, C.RUNS.find(r => r.name === s.run.name), true); return true;
    case 'ctrl+s': if (t) { buf(s, t.path).dirty = false; setMsg(s, 'Saved ' + t.path, 'done', '\u2713'); } return true;
    case 'ctrl+w': closeTab(s); return true;
    case 'ctrl+n': { let n = 1; while (s.bufs['untitled-' + n]) n++; open(s, s.fp, 'untitled-' + n); s.focus = 'pane'; return true; }
    case 'alt+,': case 'alt+.': { const p = s.panes[s.fp]; if (p.tabs.length) p.active = (p.active + (k === 'alt+.' ? 1 : p.tabs.length - 1)) % p.tabs.length; follow(s); return true; }
    case 'alt+k': if (t && s.focus === 'pane') s.popup = { type: 'hover' }; return true;
    case 'alt+/': if (t && s.focus === 'pane') { const ln = buf(s, t.path).lines[t.l]; let st = t.c; while (st > 0 && /\w/.test(ln[st - 1])) st--; s.popup = { type: 'completion', start: st, sel: 0 }; } return true;
    case 'f8': case 'shift+f8': if (t) { const ds = diagsFor(s, t.path); if (ds.length) { let i = ds.findIndex(d => d.l > t.l || (d.l === t.l && d.c0 > t.c)); if (k === 'shift+f8') { i = ds.map(d => d.l < t.l || (d.l === t.l && d.c0 < t.c)).lastIndexOf(true); if (i < 0) i = ds.length - 1; } else if (i < 0) i = 0; t.l = ds[i].l; t.c = ds[i].c0; t.sel = null; s.message = null; follow(s); } } return true;
  }
  if (/^alt\+[1-9]$/.test(k)) { const p = s.panes[s.fp], i = +k.slice(4) - 1; if (p.tabs[i]) p.active = i; follow(s); return true; }
  if (s.focus === 'tree') {
    const rows = treeRows(s);
    if (k === 'up') s.tree.sel = Math.max(0, s.tree.sel - 1); else if (k === 'down') s.tree.sel = Math.min(rows.length - 1, s.tree.sel + 1);
    else if (k === 'enter') treeActivate(s, true);
    else if (k === 'right') { const r = rows[s.tree.sel]; if (r && r.dir && !r.open) s.tree.open.add(r.path); }
    else if (k === 'left') { const r = rows[s.tree.sel]; if (r && r.dir && r.open) s.tree.open.delete(r.path); else if (r) { const parent = r.path.split('/').slice(0, -1).join('/'); selectTreePath(s, parent); } }
    else if (k === 'a' || k === 'A') { const r = rows[s.tree.sel]; const dir = r ? (r.dir ? r.path + '/' : r.path.split('/').slice(0, -1).join('/') + (r.depth ? '/' : '')) : ''; s.bar = { type: 'prompt', label: k === 'a' ? 'New file' : 'New folder', text: dir, hint: '\u23ce create   esc cancel' }; }
    else if (k === 'r') { const r = rows[s.tree.sel]; if (r) s.bar = { type: 'prompt', label: 'Rename', text: r.name, selTo: r.dir ? len(r.name) : r.name.lastIndexOf('.'), hint: '\u23ce rename   esc cancel' }; }
    else if (k === 'd') { const r = rows[s.tree.sel]; if (r) s.overlay = { type: 'confirm', kind: 'trash', file: r.name }; }
    else return false;
    return true;
  }
  if (s.focus === 'run') return false;
  if (!t) return false;
  s.message = null;
  if (/^(shift\+)?(ctrl\+)?(up|down|left|right|home|end|pgup|pgdn)$/.test(k)) { moveCursor(s, t, k); follow(s); return true; }
  if (k === 'esc') { t.sel = null; return true; }
  const b = buf(s, t.path), ln = b.lines[t.l];
  const delSel = () => { if (!t.sel) return; const a = t.sel, c = { l: t.l, c: t.c }; const [p, q] = (a.l < c.l || (a.l === c.l && a.c < c.c)) ? [a, c] : [c, a]; const head = b.lines[p.l].slice(0, p.c), tail = b.lines[q.l].slice(q.c); b.lines.splice(p.l, q.l - p.l + 1, head + tail); t.l = p.l; t.c = p.c; t.sel = null; };
  if (k === 'ctrl+a') { t.sel = { l: 0, c: 0 }; t.l = b.lines.length - 1; t.c = b.lines[t.l].length; return true; }
  if (isText(k) || k === 'space' || k === 'tab') { delSel(); const ch = k === 'space' ? ' ' : k === 'tab' ? '    ' : k; const l2 = b.lines[t.l]; b.lines[t.l] = l2.slice(0, t.c) + ch + l2.slice(t.c); t.c += ch.length; b.dirty = true; if (ch === '.') { s.popup = { type: 'completion', start: t.c, sel: 0 }; } follow(s); return true; }
  if (k === 'backspace') { if (t.sel) delSel(); else if (t.c > 0) { b.lines[t.l] = ln.slice(0, t.c - 1) + ln.slice(t.c); t.c--; } else if (t.l > 0) { const prev = b.lines[t.l - 1]; b.lines.splice(t.l - 1, 2, prev + ln); t.l--; t.c = prev.length; } b.dirty = true; follow(s); return true; }
  if (k === 'delete') { if (t.sel) delSel(); else if (t.c < ln.length) b.lines[t.l] = ln.slice(0, t.c) + ln.slice(t.c + 1); else if (t.l < b.lines.length - 1) b.lines.splice(t.l, 2, ln + b.lines[t.l + 1]); b.dirty = true; return true; }
  if (k === 'enter') { delSel(); const l2 = b.lines[t.l], ind = l2.match(/^\s*/)[0]; b.lines.splice(t.l, 1, l2.slice(0, t.c), ind + l2.slice(t.c)); t.l++; t.c = ind.length; b.dirty = true; follow(s); return true; }
  return false;
}

function mouse(s, g, x, y, type, info) {
  const h = g.find(x, y);
  if (type === 'move') { const nh = h && h.t === 'tree' && !s.overlay ? h.i : -1; if (nh !== s.tree.hover) { s.tree.hover = nh; return true; } return false; }
  if (type === 'wheel') { if (h && h.t === 'editor') { const t = activeTab(s, h.pane); if (t) { t.top = clamp(t.top + (info > 0 ? 3 : -3), 0, Math.max(0, buf(s, t.path).lines.length - 3)); return true; } } return false; }
  if (s.overlay) {
    if (!h || h.t === 'scrim') { s.overlay = null; return true; }
    const o = s.overlay;
    if (h.t === 'pick') { o.sel = h.i; return key(s, { key: 'Enter' }); }
    if (h.t === 'shit') { o.sel = h.i; return key(s, { key: 'Enter' }); }
    if (h.t === 'btn') { confirmAnswer(s, h.key); return true; }
    return false;
  }
  s.popup = null;
  if (!h) return false;
  if (h.t === 'tree') { s.focus = 'tree'; s.tree.sel = h.i; treeActivate(s, false); return true; }
  if (h.t === 'treebg') { s.focus = 'tree'; return true; }
  if (h.t === 'tab') { const p = s.panes[h.pane]; if (info === 1) { s.fp = h.pane; p.active = h.i; closeTab(s); return true; } s.fp = h.pane; s.focus = 'pane'; p.active = h.i; follow(s); return true; }
  if (h.t === 'editor') { s.fp = h.pane; s.focus = 'pane'; const t = activeTab(s, h.pane); if (t) { const lines = buf(s, t.path).lines; t.l = clamp(t.top + y - h.y0, 0, lines.length - 1); t.c = clamp(t.left + x - h.x0, 0, lines[t.l].length); t.sel = null; } return true; }
  if (h.t === 'run') { s.focus = 'run'; return true; }
  return false;
}

// ---------- scenarios
function base(W, H) {
  const s = { W, H, tree: { visible: true, open: new Set(['src', 'src/ui']), sel: 0, hover: -1, scroll: 0 }, focus: 'pane', fp: 0, split: false,
    panes: [{ tabs: [], active: 0 }], bufs: {}, overlay: null, popup: null, bar: null, run: null, lsp: 'ready', message: null, diag: true, project: 'griffin', clock: 0 };
  open(s, 0, 'src/app.rs'); open(s, 0, 'src/ui/status.rs'); open(s, 0, 'src/theme.rs');
  s.panes[0].active = 1; buf(s, 'src/app.rs').dirty = true;
  const t = activeTab(s); t.l = lineOf(s, t.path, 'pub fn render_status'); t.c = 7; t.top = 30;
  selectTreePath(s, 'src/ui/status.rs');
  return s;
}
const ST = 'src/ui/status.rs';
function at(s, sub, col, top) { const t = activeTab(s); t.l = lineOf(s, t.path, sub); t.c = col; if (top != null) t.top = top; return t; }
function withRun(s, name, n, status) { startRun(s, C.RUNS.find(r => r.name === name)); for (let i = 0; i < n; i++) tick(s); if (status) { s.run.status = status; } return s; }
function splitBase(W, H, fp) {
  const s = base(W, H); s.split = true; s.panes[1] = { tabs: [], active: 0 };
  open(s, 1, 'src/ui/mod.rs'); open(s, 1, 'src/lsp/mod.rs', { l: 4, c: 9 }); s.fp = fp; return s;
}
const SC = [
  ['1', 'Main editor', [
    ['1a', 'Editor focused', 'Tab bar per split on row 0, tree 30 cols + \u2502, editor gutter, status. Dirty app.rs shows \u25cf in tab, tree and status.', (W, H) => base(W, H)],
    ['1b', 'Tree focused + hover', 'Ctrl+E moves focus to the tree: header turns accent, the selected row is filled and bold. Mouse hover = card ground. Active file name in accent. target/ is gitignored and never listed.', (W, H) => { const s = base(W, H); s.focus = 'tree'; s.tree.hover = treeRows(s).findIndex(r => r.path === 'src/ui/find.rs'); return s; }],
  ]],
  ['2', 'Cursor line, selection, find', [
    ['2a', 'Current line', 'The cursor line gets current_line_bg across gutter + text; its number pops in gutter_active_fg bold while neighbours sit in gutter_fg.', (W, H) => { const s = base(W, H); s.diag = false; return s; }],
    ['2b', 'Selection', 'Selection keeps syntax colours on selection_bg; a selected line break shows as one cell past the line end.', (W, H) => { const s = base(W, H); s.diag = false; const t = at(s, 'let mut right', 8); t.sel = { l: t.l, c: 8 }; t.l += 3; t.c = 30; return s; }],
    ['2c', 'Find matches', 'Every match on find_match_bg; the current one on find_current_bg with find_current_fg. Enter / Shift+Enter step and wrap.', (W, H) => { const s = base(W, H); s.diag = false; at(s, 'fn counts', 4); s.bar = { type: 'find', q: 'theme', r: null, inR: false, cs: false, rx: false, cur: 0, origin: 0 }; syncFind(s); s.bar.cur += 1; jumpFind(s); follow(s); return s; }],
  ]],
  ['3', 'Vertical split', [
    ['3a', 'Left focused', 'Each split has its own tab bar. Focused split: filled active tab, cursor, current-line band. Unfocused: active tab underlined only, no band, number in text colour. Duplicate names get " \u00b7 dir".', (W, H) => splitBase(W, H, 0)],
    ['3b', 'Right focused', 'Same, focus on the right split (F6 or click).', (W, H) => { const s = splitBase(W, H, 1); return s; }],
    ['3c', 'Split at 100 cols', 'Below 120 cols a split hides the tree (Ctrl+B still toggles it back as an overlay-free column).', (W, H) => splitBase(W, H, 0), [100, 30]],
  ]],
  ['4', 'Diagnostics', [
    ['4a', 'Underlines + gutter', 'Curly underline in err / working / info. Gutter column 0 shows \u25cf in the most severe colour on that line. Status shows \u26a0 / \u2715 counts.', (W, H) => { const s = base(W, H); at(s, 'pub fn render_status', 0, 36); return s; }],
    ['4b', 'Cursor on an error', 'With the cursor inside a range, its message takes the status message slot with the severity glyph. F8 / Shift+F8 jump.', (W, H) => { const s = base(W, H); at(s, 'area.width as usize', 21, 36); return s; }],
  ]],
  ['5', 'Popups (no dim)', [
    ['5a', 'Hover (Alt+K)', 'Card anchored on the row below the cursor at its column; code block highlighted, \u251c\u2500\u2524 rule, wrapped doc (max 60 \u00d7 12).', (W, H) => { const s = base(W, H); s.diag = false; at(s, 'render_status(frame', 0); const t = at(s, 'pub fn render_status', 11); s.popup = { type: 'hover' }; follow(s); return s; }],
    ['5b', 'Completion', 'Kind column muted, label, detail right. The card is shifted left so labels line up with the typed word.', (W, H) => { const s = base(W, H); s.diag = false; const b = buf(s, ST); const l = lineOf(s, ST, 'let gap = 2;'); b.lines[l] = '        let tint = theme.'; b.dirty = true; const t = activeTab(s); t.l = l; t.c = b.lines[l].length; s.popup = { type: 'completion', start: t.c, sel: 0 }; follow(s); return s; }],
    ['5c', 'Completion filtered', 'Typing narrows by prefix (case-insensitive); the typed prefix is accent bold.', (W, H) => { const s = base(W, H); s.diag = false; const b = buf(s, ST); const l = lineOf(s, ST, 'let gap = 2;'); b.lines[l] = '        let tint = theme.bo'; const t = activeTab(s); t.l = l; t.c = b.lines[l].length; s.popup = { type: 'completion', start: t.c - 2, sel: 1 }; follow(s); return s; }],
    ['5d', 'Flipped above + left', 'Near the bottom the card flips above the cursor; near the right edge it shifts left. The cursor cell is never covered.', (W, H) => { const s = base(W, H); s.diag = false; const b = buf(s, ST); const l = lineOf(s, ST, '[left, right].concat()'); b.lines[l] = '    Line::from([left, right].concat()).style(bold.bg(theme.sidebar_bg)).patch(Style::new().fg(theme.'; const t = activeTab(s); t.l = l; t.c = b.lines[l].length; t.top = Math.max(0, l - (H - 3)); s.popup = { type: 'completion', start: t.c, sel: 0 }; follow(s); return s; }],
  ]],
  ['6', 'Dialogs (centred, dimmed)', [
    ['6a', 'Go to file', 'Ctrl+P. 3/5 width (40\u201380), \u226420 rows. Field row on card2, list, footer hints. Dir part muted, name strong.', (W, H) => { const s = base(W, H); s.overlay = { type: 'picker', kind: 'file', q: '', sel: 0 }; return s; }],
    ['6b', 'Go to file, filtered', 'Matched letters accent bold; on the selected row they become accent blocks with acc_ink.', (W, H) => { const s = base(W, H); s.overlay = { type: 'picker', kind: 'file', q: 'uist', sel: 0 }; return s; }],
    ['6c', 'Go to line', 'Ctrl+G. Digits only; out-of-range turns the value and range err.', (W, H) => { const s = base(W, H); s.overlay = { type: 'goto', text: '42' }; return s; }],
    ['6d', 'Project search', 'Alt+F. Search + Replace field rows, summary, aligned path:line column, match on find_match_bg (current row: find_current).', (W, H) => { const s = base(W, H); s.overlay = { type: 'search', q: 'theme', r: '', inR: false, cs: false, rx: false, sel: 2 }; return s; }],
    ['6e', 'Project search + replace', 'Tab into Replace: matches show underlined in err with the replacement in done after them. Alt+A asks to confirm.', (W, H) => { const s = base(W, H); s.overlay = { type: 'search', q: 'theme', r: 'palette', inR: true, cs: false, rx: false, sel: 2 }; return s; }],
    ['6f', 'Confirm replace', 'Buttons: default on tab_active_bg, others on btn; key letter underlined. \u2190\u2192/Tab move, Enter presses, letter answers, Esc cancels.', (W, H) => { const s = base(W, H); s.overlay = { type: 'confirm', kind: 'replace', n: 23, m: 5, src: { q: 'theme', r: 'palette' } }; return s; }],
    ['6g', 'Unsaved changes', 'Ctrl+W / Ctrl+Q on a dirty buffer.', (W, H) => { const s = base(W, H); s.overlay = { type: 'confirm', kind: 'unsaved', file: 'app.rs', path: 'src/app.rs' }; return s; }],
    ['6h', 'Recover backup', 'Opening a file whose backup is newer.', (W, H) => { const s = base(W, H); s.overlay = { type: 'confirm', kind: 'recover', file: 'status.rs' }; return s; }],
    ['6i', 'Move to trash', 'd in the tree.', (W, H) => { const s = base(W, H); s.focus = 'tree'; selectTreePath(s, 'src/ui/find.rs'); s.overlay = { type: 'confirm', kind: 'trash', file: 'find.rs' }; return s; }],
  ]],
  ['7', 'Bars', [
    ['7a', 'Find', 'One row above status on card2: label, text, cursor; right: Aa / .* chips (accent when on) and n/m.', (W, H) => { const s = base(W, H); s.diag = false; at(s, 'fn counts', 4); s.bar = { type: 'find', q: 'Theme', r: null, inR: false, cs: true, rx: false, cur: 0, origin: 0 }; syncFind(s); follow(s); return s; }],
    ['7b', 'Invalid regex', 'The count slot says why, in err; the pattern turns err.', (W, H) => { const s = base(W, H); s.diag = false; s.bar = { type: 'find', q: 'theme(', r: null, inR: false, cs: false, rx: true, cur: 0 }; return s; }],
    ['7c', 'Find + replace', 'Ctrl+R. Find takes half the row, \u2502, Replace up to the chips. Enter replaces current, Alt+A all (one undo step).', (W, H) => { const s = base(W, H); s.diag = false; at(s, 'fn counts', 4); s.bar = { type: 'find', q: 'theme', r: 'palette', inR: true, cs: false, rx: false, cur: 0, origin: 0 }; syncFind(s); follow(s); return s; }],
    ['7d', 'New file prompt', 'a in the tree; prefilled with the selected folder.', (W, H) => { const s = base(W, H); s.focus = 'tree'; selectTreePath(s, 'src/ui'); s.bar = { type: 'prompt', label: 'New file', text: 'src/ui/', hint: '\u23ce create   esc cancel' }; return s; }],
    ['7e', 'Rename prompt', 'r in the tree; the stem is preselected so typing replaces it and keeps the extension.', (W, H) => { const s = base(W, H); s.focus = 'tree'; s.bar = { type: 'prompt', label: 'Rename', text: 'status.rs', selTo: 6, hint: '\u23ce rename   esc cancel' }; return s; }],
  ]],
  ['8', 'Run panel', [
    ['8a', 'Running', 'F5 \u2192 picker \u2192 runs. Title row on sidebar_bg: status glyph, name, state, command, key hints. Output keeps ANSI colours mapped onto theme roles. Streams live.', (W, H) => withRun(base(W, H), 'dev', 9)],
    ['8b', 'Exited 0', '\u2713 in done.', (W, H) => withRun(base(W, H), 'cargo run', 30)],
    ['8c', 'Exited with error', '\u2715 + code in err.', (W, H) => withRun(base(W, H), 'test', 40)],
    ['8d', 'Stopped', 'Shift+F5 kills the process tree; \u25a0 in idle.', (W, H) => withRun(base(W, H), 'dev', 7, 'stopped')],
    ['8e', 'Restarted', 'Ctrl+F5 keeps old output above a dim \u2500\u2500 restarted \u2500\u2500 rule.', (W, H) => { const s = withRun(base(W, H), 'dev', 6); startRun(s, C.RUNS[0], true); tick(s); tick(s); return s; }],
    ['8f', 'Command picker', 'F5 with several commands: name, command, source (.griffin.toml or detected).', (W, H) => { const s = base(W, H); s.overlay = { type: 'picker', kind: 'run', q: '', sel: 0 }; return s; }],
    ['8g', 'Panel, nothing run', 'F4 before anything has run.', (W, H) => { const s = base(W, H); s.run = { visible: true }; s.focus = 'run'; return s; }],
  ]],
  ['9', 'Empty + edge states', [
    ['9a', 'Welcome (no buffer)', 'griffin . with nothing open: the editor shows the six keys that are not obvious. No splash on launch.', (W, H) => { const s = base(W, H); s.panes[0].tabs = []; s.tree.sel = 0; return s; }],
    ['9b', 'Untitled buffer', 'griffin alone: no project, no tree, one untitled buffer, Plain text, no server.', (W, H) => { const s = base(W, H); s.tree.visible = false; s.panes[0].tabs = []; open(s, 0, 'untitled-1'); return s; }],
    ['9c', 'Empty project', 'Tree says what to press.', (W, H) => { const s = base(W, H); s.project = 'empty'; s.panes[0].tabs = []; s.focus = 'tree'; return s; }],
    ['9d', 'File too wide', 'No wrap: horizontal scroll. \u2039 marks text hidden on the left, \u203a on the right, in muted.', (W, H) => { const s = base(W, H); s.panes[0].active = 0; s.diag = false; const t = at(s, 'Panes::compute', 150); t.top = 14; follow(s); selectTreePath(s, 'src/app.rs'); return s; }],
    ['9e', 'Server not found', 'Shown once in the message slot; the LSP slot reads \u25cb no server. Editing carries on.', (W, H) => { const s = base(W, H); s.lsp = 'notfound'; setMsg(s, 'rust: server not found (rust-analyzer)', 'working', '\u26a0'); return s; }],
    ['9f', 'Server crashed', 'Message in err; LSP slot \u2715 rust-analyzer in err; diagnostics cleared.', (W, H) => { const s = base(W, H); s.lsp = 'crashed'; setMsg(s, 'rust: server crashed (exit code 101)', 'err', '\u2715'); return s; }],
    ['9g', 'Below minimum', 'Under 100\u00d730 Griffin draws only this.', (W, H) => base(W, H), [92, 26]],
  ]],
  ['10', 'Resize', [
    ['10a', '100\u00d730 minimum', 'Tree 22 cols (19% of width, 22\u201336). Run panel 9 rows (30%, 6\u201318). Status drops LSP name, then language.', (W, H) => withRun(base(W, H), 'dev', 8), [100, 30]],
    ['10b', '160\u00d745 design size', 'Tree 30, split 64 / 64, run 14 rows.', (W, H) => { const s = withRun(splitBase(W, H, 0), 'dev', 8); return s; }, [160, 45]],
    ['10c', '220\u00d760 large', 'Tree caps at 36, run panel caps at 18 rows; extra width goes to the editors.', (W, H) => { const s = withRun(splitBase(W, H, 0), 'test', 40); return s; }, [220, 60]],
  ]],
];
const SCEN = {}; SC.forEach(([gid, gname, items]) => items.forEach(([id, label, note, make, size]) => SCEN[id] = { id, label, note, make: (W, H) => { const s = make(W, H); follow(s); return s; }, size, group: gid + ' \u00b7 ' + gname }));
function showcase(W, H) { const s = withRun(base(W, H), 'test', 40); s.bar = { type: 'find', q: 'theme', r: null, inR: false, cs: false, rx: false, cur: 0, origin: 0 }; at(s, 'fn counts', 4); syncFind(s); s.bar.cur += 1; jumpFind(s); const t = activeTab(s); t.top = Math.max(0, t.l - 6); follow(s); return s; }

window.Griffin = { render, toHTML, key, mouse, tick, follow, layout, scenarios: SC, scenario: id => SCEN[id], showcase, theme: T.theme, themes: T.NAMES, roles: T.ROLES, keyName, internals: { Grid, tokRust, treeRows, fuzzy, TREE } };
})();
