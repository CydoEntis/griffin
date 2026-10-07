// Griffin themes: Hydra's 11 palettes (from src/theme.rs) + the proposed new roles.
(function () {
'use strict';
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const rgb = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
const toHex = a => '#' + a.map(v => clamp(Math.round(v), 0, 255).toString(16).padStart(2, '0')).join('');
const mix = (a, b, t) => { const x = rgb(a), y = rgb(b); return toHex(x.map((v, i) => v + (y[i] - v) * t)); };
const lum = h => { const [r, g, b] = rgb(h); return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255; };

const NAMES = ['hydra', 'papercolor-dark', 'tango-dark', 'monokai', 'tokyo-night', 'catppuccin-mocha', 'catppuccin-latte', 'gruvbox', 'nord', 'dracula', 'mono'];
const SYN = ['keyword', 'string', 'comment', 'function', 'type', 'number', 'constant', 'operator', 'punctuation', 'variable', 'property', 'tag', 'attribute'];

// Hydra design order: bg fg surf card card2 line dim text strong acc accInk needs ok err btn hov
const DESIGN = {
  'hydra': [["#070b10","#c9d1d9","#0c131b","#0f1821","#18242f","#1f2c3a","#71808f","#a7b4c2","#f2f6f8","#c3f53c","#0a1204","#ffb547","#7fd962","#ff6b6b","#1d2a37","#2a3a4c"],
    ["#a593ff","#7fd962","#71808f","#5aa9ff","#3dd6c0","#ffb547","#ffb547","#ff7ab6","#a7b4c2","#c9d1d9","#e8c565","#ff7ab6","#e8c565"]],
  'papercolor-dark': [["#1c1c1c","#d0d0d0","#262626","#303030","#3a3a3a","#444444","#808080","#b2b2b2","#eeeeee","#00afaf","#1c1c1c","#ffaf00","#5faf00","#ff5f87","#3a3a3a","#5f5faf"],
    ["#ff5faf","#d7af5f","#808080","#5fafd7","#af87d7","#ffaf00","#ffaf00","#00afaf","#b2b2b2","#d0d0d0","#5f8787","#ff5f87","#d7af5f"]],
  'tango-dark': [["#2e3436","#eeeeec","#252a2b","#363c3e","#41474a","#555753","#9a9c97","#d3d7cf","#eeeeec","#8ae234","#2e3436","#fcaf3e","#73d216","#ff5c5c","#4a5052","#204a87"],
    ["#ad7fa8","#8ae234","#888a85","#729fcf","#34e2e2","#fcaf3e","#fcaf3e","#fce94f","#d3d7cf","#eeeeec","#e9b96e","#ef2929","#fce94f"]],
  'monokai': [["#272822","#f8f8f2","#1e1f1c","#2f302a","#3e3d32","#49483e","#8f8a72","#cfcfc2","#f8f8f2","#a6e22e","#272822","#fd971f","#a6e22e","#f92672","#3e3d32","#55544a"],
    ["#f92672","#e6db74","#75715e","#a6e22e","#66d9ef","#ae81ff","#ae81ff","#f92672","#cfcfc2","#f8f8f2","#fd971f","#f92672","#a6e22e"]],
  'tokyo-night': [["#1a1b26","#c0caf5","#16161e","#1f2335","#292e42","#292e42","#7a82ad","#a9b1d6","#e0e6ff","#7aa2f7","#1a1b26","#e0af68","#9ece6a","#f7768e","#292e42","#3b4261"],
    ["#bb9af7","#9ece6a","#565f89","#7aa2f7","#7dcfff","#ff9e64","#ff9e64","#89ddff","#a9b1d6","#c0caf5","#73daca","#f7768e","#e0af68"]],
};
// Classic order: bg fg muted accent border border_active sidebar_bg selection_bg tab_active_bg tab_active_fg working err done
const CLASSIC = {
  'catppuccin-mocha': [["#1e1e2e","#cdd6f4","#6c7086","#89b4fa","#45475a","#89b4fa","#181825","#313244","#89b4fa","#1e1e2e","#f9e2af","#f38ba8","#a6e3a1"],
    ["#cba6f7","#a6e3a1","#9399b2","#89b4fa","#f9e2af","#fab387","#fab387","#89dceb","#9399b2","#cdd6f4","#b4befe","#f38ba8","#f9e2af"]],
  'catppuccin-latte': [["#eff1f5","#303446","#6c6f85","#7a2fd8","#bcc0cc","#7a2fd8","#e6e9ef","#ccd0da","#7a2fd8","#eff1f5","#8a5a00","#d20f39","#2f8a1f"],
    ["#8839ef","#40a02b","#7c7f93","#1e66f5","#df8e1d","#fe640b","#fe640b","#04a5e5","#7c7f93","#303446","#7287fd","#d20f39","#df8e1d"]],
  'gruvbox': [["#282828","#ebdbb2","#928374","#fabd2f","#504945","#fabd2f","#1d2021","#3c3836","#fabd2f","#282828","#fe8019","#fb4934","#b8bb26"],
    ["#fb4934","#b8bb26","#928374","#8ec07c","#fabd2f","#d3869b","#d3869b","#fe8019","#a89984","#ebdbb2","#83a598","#fe8019","#fabd2f"]],
  'nord': [["#2e3440","#eceff4","#8390a8","#a3be8c","#434c5e","#a3be8c","#272c36","#3b4252","#a3be8c","#2e3440","#ebcb8b","#e0707a","#a3be8c"],
    ["#81a1c1","#a3be8c","#8390a8","#88c0d0","#8fbcbb","#b48ead","#b48ead","#81a1c1","#d8dee9","#d8dee9","#88c0d0","#81a1c1","#8fbcbb"]],
  'dracula': [["#282a36","#f8f8f2","#7a8ac0","#bd93f9","#44475a","#bd93f9","#21222c","#44475a","#bd93f9","#282a36","#f1fa8c","#ff5555","#50fa7b"],
    ["#ff79c6","#f1fa8c","#7a8ac0","#50fa7b","#8be9fd","#bd93f9","#bd93f9","#ff79c6","#f8f8f2","#f8f8f2","#ffb86c","#ff79c6","#50fa7b"]],
};
// mono uses the terminal's own colours; previewed with Windows Terminal "Campbell".
const ANSI = { Reset: null, Black: '#0c0c0c', DarkGray: '#767676', Gray: '#cccccc', White: '#f2f2f2', Yellow: '#c19c00', Green: '#13a10e', Red: '#c50f1f', Blue: '#3b78ff' };

const ROLES = [
  ['bg', 'Editor ground, run-panel output'], ['fg', 'Editor text'], ['muted', 'Hints, markers, kind labels (alias dim, idle)'],
  ['accent', 'Focus, matched letters, active file in tree (alias acc)'], ['border', 'Popup card border'], ['border_active', 'Reserved (no pane boxes)'],
  ['sidebar_bg', 'Chrome: tab bars, tree, status, run title (alias surf)'], ['selection_bg', 'Selected text'],
  ['tab_active_bg', 'Focused split\u2019s active tab, default button'], ['tab_active_fg', 'Text on tab_active_bg'],
  ['card', 'Dialog + popup ground'], ['card2', 'Input rows: bars, dialog fields'], ['btn', 'Non-default buttons'],
  ['hov', 'Keyboard row in lists, tree selection'], ['line', '\u2502 dividers'], ['text', 'Chrome text'], ['strong', 'Names, titles, field text'],
  ['acc_ink', 'Text on accent'], ['err', 'Errors, failed runs (alias blocked)'], ['working', 'Warnings, running, dirty \u25cf (alias warning, needs)'],
  ['done', 'Ready server, exit 0 (alias ok)'], ['idle', 'Stopped run (alias of muted)'],
  ['current_line_bg', 'NEW \u00b7 cursor line behind gutter + text'], ['gutter_fg', 'NEW \u00b7 line numbers (dimmer than muted)'],
  ['gutter_active_fg', 'NEW \u00b7 cursor line number, bold'], ['find_match_bg', 'NEW \u00b7 every find match'],
  ['find_current_bg', 'NEW \u00b7 the current find match'], ['find_current_fg', 'NEW \u00b7 text on the current match'],
  ['info', 'NEW \u00b7 info/hint diagnostics'], ['scrim', 'NEW \u00b7 dim target behind dialogs (mix 0.6)'],
];
const NEW_ROLES = ROLES.filter(r => r[1].startsWith('NEW')).map(r => r[0]);

function fromDesign(c) {
  return { bg: c[0], fg: c[1], sidebar_bg: c[2], card: c[3], card2: c[4], border: c[5], line: c[5], muted: c[6], text: c[7], strong: c[8],
    accent: c[9], border_active: c[9], tab_active_bg: c[9], tab_active_fg: c[10], acc_ink: c[10], working: c[11], done: c[12], err: c[13], btn: c[14], hov: c[15], selection_bg: c[15] };
}
function fromClassic(c) {
  const [bg, fg, muted] = c;
  return { bg, fg, muted, accent: c[3], border: c[4], border_active: c[5], sidebar_bg: c[6], selection_bg: c[7], tab_active_bg: c[8], tab_active_fg: c[9],
    working: c[10], err: c[11], done: c[12], card: mix(bg, fg, .05), card2: mix(bg, fg, .11), btn: mix(bg, fg, .14), hov: c[7], line: c[4], text: mix(muted, fg, .5), strong: fg, acc_ink: bg };
}
const cache = {};
function theme(name) {
  if (cache[name]) return cache[name];
  let r, mods = {}, names = null, mono = false;
  if (DESIGN[name] || CLASSIC[name]) {
    const [c, s] = DESIGN[name] || CLASSIC[name];
    r = DESIGN[name] ? fromDesign(c) : fromClassic(c);
    SYN.forEach((k, i) => r['syn.' + k] = s[i]);
  } else {
    mono = true;
    names = { bg: 'Reset', fg: 'Reset', muted: 'DarkGray', accent: 'White', border: 'DarkGray', border_active: 'White', sidebar_bg: 'Reset', selection_bg: 'DarkGray',
      tab_active_bg: 'White', tab_active_fg: 'Black', working: 'Yellow', done: 'Green', err: 'Red', card: 'Black', card2: 'DarkGray', btn: 'DarkGray', hov: 'DarkGray',
      line: 'DarkGray', text: 'Gray', strong: 'White', acc_ink: 'Black',
      current_line_bg: 'Reset', gutter_fg: 'DarkGray', gutter_active_fg: 'White', find_match_bg: 'DarkGray', find_current_bg: 'Yellow', find_current_fg: 'Black', info: 'Blue', scrim: 'DIM modifier' };
    r = {};
    for (const k in names) r[k] = names[k] === 'Reset' ? (k.endsWith('bg') ? '#0c0c0c' : '#cccccc') : (ANSI[names[k]] || '#000000');
    const bold = ['keyword', 'function', 'type', 'constant', 'tag'], dim = ['comment', 'punctuation', 'attribute'];
    SYN.forEach(k => { r['syn.' + k] = '#cccccc'; names['syn.' + k] = 'Reset' + (bold.includes(k) ? ' + BOLD' : dim.includes(k) ? ' + DIM' : ''); if (bold.includes(k)) mods['syn.' + k] = { b: 1 }; if (dim.includes(k)) mods['syn.' + k] = { d: 1 }; });
  }
  r.idle = r.muted; if (names) names.idle = names.muted;
  const dark = lum(r.bg) < .5;
  if (!mono) {
    r.current_line_bg = mix(r.bg, r.fg, .06);
    r.gutter_fg = mix(r.muted, r.bg, .3);
    r.gutter_active_fg = r.strong;
    r.find_match_bg = mix(r.bg, r.working, dark ? .3 : .32);
    r.find_current_bg = r.working;
    r.find_current_fg = r.bg;
    r.info = r['syn.function'];
    r.scrim = dark ? mix(r.bg, '#000000', .5) : mix(r.bg, r.fg, .3);
  }
  // ANSI colours from run output map onto theme roles.
  Object.assign(r, { 'ansi.red': r.err, 'ansi.green': r.done, 'ansi.yellow': r.working, 'ansi.blue': r['syn.function'], 'ansi.magenta': r['syn.keyword'], 'ansi.cyan': r['syn.type'], 'ansi.white': r.strong, 'ansi.black': r.muted });
  return cache[name] = { name, r, mods, mono, dark, names };
}
window.GriffinThemes = { NAMES, SYN, ROLES, NEW_ROLES, theme, mix, clamp, lum, DESIGN, CLASSIC };
})();
