// Condor's PDF view: PDF files, and the PDFs LibreOffice makes from Office files, drawn with the
// bundled pdf.js (ui/vendor/pdfjs). Unlike the browser's own PDF viewer, the pages scroll in
// this window, so Condor's ui/content.js keeps each file's position (and pages, Esc...) as in
// any other document. Pages are laid out at once (so the height is right for restoring the
// position) and drawn as they come near the view; far ones are let go again.
import * as pdfjs from "/__condor/pdfjs/pdf.min.mjs";

pdfjs.GlobalWorkerOptions.workerSrc = "/__condor/pdfjs/pdf.worker.min.mjs";
// Building: ui/content.js holds a bookmark's goto until again().
window.__condorBusy = true;

const out = document.getElementById("pages");
const msg = document.getElementById("msg");
const GAP = 16;         // around and between pages
const TOP = 8;          // where a page's top sits when shown from its top
const ACTUAL = 96 / 72; // CSS px per PDF point at 100%
const zoomKey = "condor:pdfzoom:" + location.pathname + location.search;
let zoom = 1;           // the reader's zoom, relative to fitting the width (at most 100%)
try { zoom = +sessionStorage.getItem(zoomKey) || 1; } catch (e) {}
let scale = 1;          // CSS px per point, now
let pdf = null;
const pages = [];       // {n, w, h (points), div, canvas, text, links, drawn (scale), task}

function fail(title, text) {
  window.__condorBusy = false;
  out.innerHTML = "";
  msg.hidden = false;
  msg.innerHTML = "";
  const h = document.createElement("h2");
  h.textContent = title;
  const p = document.createElement("p");
  p.textContent = text;
  msg.append(h, p, edgeLink("Open it in Edge's PDF viewer"));
}

// The browser's own viewer for this PDF (Back returns here).
function edgeLink(label) {
  const a = document.createElement("a");
  a.href = location.pathname + "?" + window.PDF_SRC;
  a.textContent = label;
  a.title = "Edge's PDF viewer: its own toolbar (print, draw...), but Condor can't keep your place in it. Back returns.";
  return a;
}

function room() {
  const toc = document.getElementById("toc");
  return document.documentElement.clientWidth - (toc && !toc.hidden ? toc.offsetWidth : 0) - 2 * GAP;
}

// Size every page for the current scale, keeping the reader's place.
function layout() {
  const widest = Math.max(...pages.map((p) => p.w));
  const fit = Math.min(ACTUAL, Math.max(80, room()) / widest);
  const next = fit * zoom;
  const anchor = place();
  scale = next;
  for (const p of pages) {
    p.div.style.width = Math.floor(p.w * scale) + "px";
    p.div.style.height = Math.floor(p.h * scale) + "px";
    p.div.style.setProperty("--scale-factor", scale);
  }
  zoomLabel.textContent = Math.round(scale / ACTUAL * 100) + "%";
  if (anchor) goTo(anchor.i, anchor.f * pages[anchor.i].h);
  redraw();
}

// Where the view is: the page at the top of the window and how far into it (0..1).
function place() {
  if (!pages.length || !pages[0].div.offsetHeight) return null;
  let i = 0;
  pages.forEach((p, j) => { if (p.div.getBoundingClientRect().top <= TOP + 1) i = j; });
  const r = pages[i].div.getBoundingClientRect();
  return {i, f: Math.max(0, (TOP - r.top) / r.height)};
}

// Scroll so page i shows from `y` points down it (its top by default).
function goTo(i, y = 0) {
  const p = pages[Math.max(0, Math.min(pages.length - 1, i))];
  if (!p) return;
  scrollBy({top: p.div.getBoundingClientRect().top - TOP + y * scale, behavior: "instant"});
}

// Draw page p at the current scale (once; the promise settles when its text is there too).
function draw(p) {
  if (p.drawn === scale) return p.drawing;
  p.drawing = drawNow(p);
  return p.drawing;
}

async function drawNow(p) {
  const s = scale;
  p.drawn = s;
  if (p.task) p.task.cancel();
  const page = await pdf.getPage(p.n);
  if (p.drawn !== s) return;
  const vp = page.getViewport({scale: s});
  const dpr = window.devicePixelRatio || 1;
  const c = document.createElement("canvas");
  c.width = Math.floor(vp.width * dpr);
  c.height = Math.floor(vp.height * dpr);
  p.task = page.render({canvasContext: c.getContext("2d"), viewport: vp, transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : null});
  try {
    await p.task.promise;
  } catch (e) {
    return; // cancelled: a newer drawing (or letting go) took over
  } finally {
    p.task = null;
  }
  if (p.drawn !== s) return;
  if (p.canvas) p.canvas.replaceWith(c); else p.div.prepend(c);
  p.canvas = c;
  // Selectable text over the picture.
  const t = document.createElement("div");
  t.className = "textLayer";
  const tl = new pdfjs.TextLayer({textContentSource: page.streamTextContent({includeMarkedContent: true, disableNormalization: true}), container: t, viewport: vp});
  try {
    await tl.render();
  } catch (e) {}
  if (p.drawn !== s) return;
  if (p.text) p.text.replaceWith(t); else p.div.append(t);
  p.text = t;
  // One span per text item, in order: what search matches map onto.
  p.spans = tl.textDivs;
  paintFinds();
  // Links: within the document (contents pages) and out of it.
  const l = document.createElement("div");
  l.className = "links";
  for (const a of await page.getAnnotations({intent: "display"})) {
    if (a.subtype !== "Link" || (!a.url && !a.dest)) continue;
    const [x1, y1, x2, y2] = vp.convertToViewportRectangle(a.rect);
    const e = document.createElement("a");
    Object.assign(e.style, {left: Math.min(x1, x2) + "px", top: Math.min(y1, y2) + "px", width: Math.abs(x2 - x1) + "px", height: Math.abs(y2 - y1) + "px"});
    if (a.url) {
      e.href = a.url;
      e.title = a.url;
    } else {
      e.href = "#";
      e.onclick = (ev) => { ev.preventDefault(); toDest(a.dest); };
    }
    l.append(e);
  }
  if (p.links) p.links.replaceWith(l); else p.div.append(l);
  p.links = l;
}

function letGo(p) {
  if (!p.drawn) return;
  p.drawn = 0;
  p.spans = null;
  if (p.task) p.task.cancel();
  for (const k of ["canvas", "text", "links"]) {
    if (p[k]) { p[k].remove(); p[k] = null; }
  }
}

// Draw the pages near the view; let go of far ones (a long PDF would otherwise hold every page).
function redraw() {
  const h = innerHeight;
  for (const p of pages) {
    const r = p.div.getBoundingClientRect();
    if (r.bottom > -h && r.top < 2 * h) draw(p);
    else if (r.bottom < -4 * h || r.top > 5 * h) letGo(p);
  }
}

// A destination (named, or [page ref, /XYZ, left, top, zoom]): its page index and height in points.
async function resolve(dest) {
  if (typeof dest === "string") dest = await pdf.getDestination(dest);
  if (!Array.isArray(dest)) return null;
  const ref = dest[0];
  const i = typeof ref === "object" && ref ? await pdf.getPageIndex(ref) : +ref;
  if (!(i >= 0 && i < pages.length)) return null;
  let y = 0;
  const top = dest[1] && dest[1].name === "XYZ" ? dest[3] : dest[1] && /^FitB?H$/.test(dest[1].name) ? dest[2] : null;
  if (typeof top === "number") y = Math.max(0, pages[i].top - top);
  return {i, y};
}

async function toDest(dest) {
  try {
    const d = await resolve(dest);
    if (d) goTo(d.i, d.y);
  } catch (e) {}
}

// The document's bookmarks as a contents panel (☰), like the built-in Word view's.
async function contents() {
  const outline = await pdf.getOutline();
  if (!outline || !outline.length) return;
  const items = [];
  (function walk(list, level) {
    for (const o of list) {
      items.push({title: o.title, dest: o.dest, url: o.url, level});
      if (o.items && o.items.length) walk(o.items, level + 1);
    }
  })(outline, 0);
  await Promise.all(items.map(async (h) => { try { h.at = h.dest && await resolve(h.dest); } catch (e) {} }));
  const btn = document.createElement("button");
  btn.id = "toc-btn";
  btn.title = "Contents";
  btn.textContent = "☰";
  const panel = document.createElement("nav");
  panel.id = "toc";
  panel.hidden = true;
  const head = document.createElement("div");
  head.className = "toc-head";
  head.textContent = "Contents";
  const close = document.createElement("button");
  close.textContent = "×";
  close.title = "Close";
  head.append(close);
  panel.append(head);
  const links = items.map((h) => {
    const a = document.createElement("a");
    a.style.paddingLeft = 10 + 14 * h.level + "px";
    a.className = "l" + h.level;
    a.textContent = h.title;
    if (h.url) a.href = h.url;
    else a.onclick = (e) => { e.preventDefault(); if (h.at) goTo(h.at.i, h.at.y); };
    panel.append(a);
    return a;
  });
  let current = -1;
  const mark = () => {
    if (panel.hidden) return;
    let i = -1;
    items.forEach((h, j) => {
      if (!h.at) return;
      const top = pages[h.at.i].div.getBoundingClientRect().top + h.at.y * scale;
      if (top < TOP + 12) i = j;
    });
    if (i === current) return;
    if (current >= 0) links[current].classList.remove("cur");
    if (i >= 0) { links[i].classList.add("cur"); links[i].scrollIntoView({block: "nearest"}); }
    current = i;
  };
  const show = (on) => {
    panel.hidden = !on;
    document.body.classList.toggle("toc-open", on);
    btn.classList.toggle("on", on);
    try { localStorage.setItem("condor-pdf-toc", on ? "1" : "0"); } catch (e) {}
    layout();
    if (on) mark();
  };
  btn.onclick = () => show(panel.hidden);
  close.onclick = () => show(false);
  addEventListener("scroll", mark, {passive: true});
  document.body.append(panel, btn);
  let saved = null;
  try { saved = localStorage.getItem("condor-pdf-toc"); } catch (e) {}
  if (saved === "1") show(true);
}

// Zoom: − / + in the corner, Ctrl+wheel, Ctrl+= / Ctrl+- (Ctrl+0: fit again).
const bar = document.createElement("div");
bar.id = "bar";
const button = (text, title, f) => {
  const b = document.createElement("button");
  b.textContent = text;
  b.title = title;
  b.onclick = f;
  bar.append(b);
  return b;
};
const pageLabel = document.createElement("span");
pageLabel.className = "pg";
bar.append(pageLabel);
button("−", "Zoom out (Ctrl+-)", () => setZoom(zoom / 1.2));
const zoomLabel = button("", "Fit the width (Ctrl+0)", () => setZoom(1));
button("+", "Zoom in (Ctrl+=)", () => setZoom(zoom * 1.2));
function fbtnSearch() {
  const b = document.createElement("button");
  b.textContent = "\u2315";
  b.title = "Find (Ctrl+F)";
  b.className = "search";
  b.onclick = openFind;
  return b;
}
function setZoom(z) {
  zoom = Math.max(0.2, Math.min(8, z));
  if (Math.abs(zoom - 1) < 0.02) zoom = 1;
  try { sessionStorage.setItem(zoomKey, String(zoom)); } catch (e) {}
  layout();
}
addEventListener("wheel", (e) => {
  if (!e.ctrlKey) return;
  e.preventDefault();
  setZoom(zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1));
}, {passive: false});
addEventListener("keydown", (e) => {
  if (!e.ctrlKey || e.altKey || e.metaKey) return;
  if (e.key === "=" || e.key === "+") setZoom(zoom * 1.2);
  else if (e.key === "-") setZoom(zoom / 1.2);
  else if (e.key === "0") setZoom(1);
  else return;
  e.preventDefault();
}, true);

// ----- Search (Ctrl+F): every match highlighted, the current one stronger; Enter / F3 for the
// next, Shift+Enter / Shift+F3 for the one before, Esc to close.

// A page's text as pdf.js gives it, item by item (the same items its text layer makes spans
// of), joined into one string to search, with a space where a line ends.
async function pageText(p) {
  if (p.textItems) return p.textItems;
  const page = await pdf.getPage(p.n);
  const tc = await page.getTextContent({disableNormalization: true});
  const items = tc.items.filter((it) => it.str !== undefined);
  let all = "";
  const starts = [];
  for (const it of items) {
    starts.push(all.length);
    all += it.str;
    if (it.hasEOL && !/\s$/.test(it.str)) all += " ";
  }
  p.textItems = {items, starts, lower: all.toLowerCase()};
  return p.textItems;
}

// Offset in a page's joined text -> [item index, offset within its text].
function itemAt(t, off) {
  let lo = 0, hi = t.starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (t.starts[mid] <= off) lo = mid; else hi = mid - 1;
  }
  return [lo, Math.min(off - t.starts[lo], t.items[lo].str.length)];
}

let finds = [];      // {p (page), start, end}
let findAt = -1;     // the current one
let findQuery = "";
let findRun = 0;

async function search(q) {
  const run = ++findRun;
  findQuery = q;
  finds = [];
  findAt = -1;
  const needle = q.toLowerCase().replace(/\s+/g, " ");
  if (!needle.trim()) { paintFinds(); findCount(); return; }
  const from = place();
  for (const p of pages) {
    const t = await pageText(p);
    if (run !== findRun) return;
    for (let i = t.lower.indexOf(needle); i >= 0; i = t.lower.indexOf(needle, i + 1)) {
      finds.push({p, start: i, end: i + needle.length});
    }
    findCount(true);
  }
  // The first match from where the reader is.
  const here = from ? from.i : 0;
  findAt = finds.findIndex((f) => f.p.n - 1 >= here);
  if (findAt < 0 && finds.length) findAt = 0;
  findCount();
  paintFinds();
  if (findAt >= 0) showFind();
}

function rangeOf(f) {
  const t = f.p.textItems, spans = f.p.spans;
  if (!t || !spans) return null;
  const [a, ao] = itemAt(t, f.start), [b, bo] = itemAt(t, f.end);
  const na = spans[a] && spans[a].firstChild, nb = spans[b] && spans[b].firstChild;
  if (!na || !nb) return null;
  const r = new Range();
  try {
    r.setStart(na, Math.min(ao, na.length));
    r.setEnd(nb, Math.min(bo, nb.length));
  } catch (e) {
    return null;
  }
  return r;
}

// Highlight the matches on the pages drawn now.
function paintFinds() {
  if (!window.CSS || !CSS.highlights) return;
  const all = [], cur = [];
  finds.forEach((f, i) => {
    if (!f.p.spans) return;
    const r = rangeOf(f);
    if (r) (i === findAt ? cur : all).push(r);
  });
  CSS.highlights.set("find", new Highlight(...all));
  CSS.highlights.set("find-cur", new Highlight(...cur));
}

async function showFind() {
  const f = finds[findAt];
  if (!f) return;
  const r0 = f.p.div.getBoundingClientRect();
  if (r0.bottom < 0 || r0.top > innerHeight) goTo(f.p.n - 1);
  await draw(f.p);
  if (finds[findAt] !== f) return;
  paintFinds();
  const r = rangeOf(f);
  if (!r) return;
  const box = r.getBoundingClientRect();
  if (box.top < 60 || box.bottom > innerHeight - 20) scrollBy({top: box.top - innerHeight / 3, behavior: "instant"});
  if (box.left < 0 || box.right > innerWidth) scrollBy({left: box.left - innerWidth / 3, behavior: "instant"});
}

function step(dir) {
  if (!finds.length) return;
  findAt = (findAt + dir + finds.length) % finds.length;
  findCount();
  showFind();
}

const findBox = document.createElement("div");
findBox.id = "find";
findBox.hidden = true;
const findInput = document.createElement("input");
findInput.placeholder = "Find in document";
findInput.spellcheck = false;
const findLabel = document.createElement("span");
findLabel.className = "count";
const fbtn = (text, title, f) => {
  const b = document.createElement("button");
  b.textContent = text;
  b.title = title;
  b.onclick = f;
  return b;
};
findBox.append(findInput, findLabel,
  fbtn("\u2191", "Previous (Shift+Enter)", () => step(-1)),
  fbtn("\u2193", "Next (Enter)", () => step(1)),
  fbtn("\u00d7", "Close (Esc)", () => closeFind()));
function findCount(busy) {
  findLabel.textContent = !findQuery.trim() ? "" : finds.length ? (findAt >= 0 ? findAt + 1 + " of " : "") + finds.length + (busy ? "\u2026" : "")
    : busy ? "\u2026" : "None";
}
let findTimer = null;
findInput.addEventListener("input", () => {
  clearTimeout(findTimer);
  findTimer = setTimeout(() => search(findInput.value), 150);
});
findInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === "F3") {
    e.preventDefault();
    if (findInput.value !== findQuery) search(findInput.value); else step(e.shiftKey ? -1 : 1);
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation(); // not back to the tree: just close this
    closeFind();
  }
});
function openFind() {
  findBox.hidden = false;
  const sel = String(getSelection() || "").trim();
  if (sel && sel.length < 200 && !sel.includes("\n")) findInput.value = sel;
  findInput.focus();
  findInput.select();
  if (findInput.value && findInput.value !== findQuery) search(findInput.value);
}
function closeFind() {
  findBox.hidden = true;
  findRun++;
  finds = [];
  findQuery = "";
  paintFinds();
  findInput.blur();
}
addEventListener("keydown", (e) => {
  if (e.ctrlKey && !e.altKey && !e.shiftKey && (e.key === "f" || e.key === "F")) {
    e.preventDefault();
    openFind();
  } else if (e.key === "F3" && !e.ctrlKey && !e.altKey && document.activeElement !== findInput) {
    e.preventDefault();
    if (findBox.hidden) openFind(); else step(e.shiftKey ? -1 : 1);
  }
}, true);

// ----- Bookmarks (ui/content.js asks): a place is a page and points down it, so it holds at
// any zoom.
window.__condorLocate = () => {
  if (!pages.length) return null;
  const s = getSelection();
  let i, py;
  if (s && !s.isCollapsed) {
    const r = s.getRangeAt(0).getBoundingClientRect();
    i = 0;
    pages.forEach((p, j) => { if (p.div.getBoundingClientRect().top <= r.top + 1) i = j; });
    py = Math.max(0, (r.top - pages[i].div.getBoundingClientRect().top) / scale);
  } else {
    const a = place() || {i: 0, f: 0};
    i = a.i;
    py = a.f * pages[i].h;
  }
  return {page: i + 1, py: Math.round(py), frac: (i + py / pages[i].h) / pages.length, label: "p. " + (i + 1)};
};
window.__condorGoto = (b) => {
  if (!pages.length) return false;
  const at = b.at || {};
  let i, py;
  if (at.page) {
    i = Math.min(pages.length, at.page) - 1;
    py = at.py || 0;
  } else {
    // Made in another view (the built-in Word view): as far through.
    const x = Math.max(0, Math.min(0.9999, b.frac || 0)) * pages.length;
    i = Math.floor(x);
    py = (x - i) * pages[i].h;
  }
  goTo(i, Math.max(0, py - innerHeight / 3 / scale));
  if (b.text && at.sel !== null) selectText(i, b.text);
  return true;
};
// Select `text` on page i (or the next or one before), once drawn.
async function selectText(i, text) {
  const want = text.toLowerCase().replace(/\s+/g, " ").trim();
  if (!want) return;
  for (const j of [i, i + 1, i - 1]) {
    const p = pages[j];
    if (!p) continue;
    const t = await pageText(p);
    let k = t.lower.indexOf(want), len = want.length;
    if (k < 0) { k = t.lower.indexOf(want.slice(0, 60)); len = Math.min(want.length, t.lower.length - k); }
    if (k < 0) continue;
    await draw(p);
    const r = rangeOf({p, start: k, end: k + len});
    if (!r) return;
    const s = getSelection();
    s.removeAllRanges();
    s.addRange(r);
    const box = r.getBoundingClientRect();
    if (box.top < 40 || box.bottom > innerHeight - 20) scrollBy({top: box.top - innerHeight / 3, behavior: "instant"});
    return;
  }
}

// The page mostly in view: the last one starting above a third of the way down (or the last
// page, at the very bottom).
function showPage() {
  if (!pages.length) return;
  let n = 1;
  pages.forEach((p, j) => { if (p.div.getBoundingClientRect().top < innerHeight / 3) n = j + 1; });
  const el = document.scrollingElement;
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 2) n = pages.length;
  pageLabel.textContent = n + " / " + pages.length;
}

(async () => {
  try {
    const res = await fetch(location.pathname + "?" + window.PDF_SRC);
    if (!res.ok) throw new Error("reading the file failed (" + res.status + ")");
    pdf = await pdfjs.getDocument({data: new Uint8Array(await res.arrayBuffer()), isEvalSupported: false}).promise;
  } catch (e) {
    const pw = e && e.name === "PasswordException";
    fail("Can't show this PDF here", pw ? "It is protected by a password." : (e && e.message) || String(e));
    return;
  }
  // Every page's size first, so the full height is there to restore the position into.
  for (let n = 1; n <= pdf.numPages; n++) {
    const page = await pdf.getPage(n);
    const [x0, y0, x1, y1] = page.view;
    const rot = page.rotate % 180 !== 0;
    const w = x1 - x0, h = y1 - y0;
    const div = document.createElement("div");
    div.className = "page";
    pages.push({n, w: rot ? h : w, h: rot ? w : h, top: y1, div, drawn: 0});
    out.append(div);
  }
  msg.hidden = true;
  bar.append(edgeLink("Edge"));
  bar.insertBefore(fbtnSearch(), bar.firstChild);
  document.body.append(bar, findBox);
  layout();
  await contents().catch(() => {});
  addEventListener("resize", () => layout());
  addEventListener("scroll", () => { redraw(); showPage(); }, {passive: true});
  // Back where the reader was (ui/content.js), else at a page asked for (#page=N, from the
  // built-in Word view).
  window.__condorBusy = false;
  const want = /^#page=(\d+)/.exec(location.hash);
  if (window.__condor && window.__condor.hasSession()) window.__condor.again();
  else if (want) goTo(+want[1] - 1);
  else if (window.__condor) window.__condor.again();
  redraw();
  showPage();
  // Left/Right, Ctrl+PgUp/PgDn (ui/content.js): to the next page's top, or back to this
  // page's top, then the one before.
  window.__condorPage = (dir) => {
    const a = place();
    if (!a) return;
    const at = pages[a.i].div.getBoundingClientRect().top - TOP;
    goTo(dir > 0 ? a.i + 1 : at < -2 ? a.i : a.i - 1);
  };
})();
