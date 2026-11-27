// Condor's PDF view: PDF files, and the PDFs LibreOffice makes from Office files, drawn with the
// bundled pdf.js (ui/vendor/pdfjs). Unlike the browser's own PDF viewer, the pages scroll in
// this window, so Condor's ui/content.js keeps each file's position (and pages, Esc...) as in
// any other document. Pages are laid out at once (so the height is right for restoring the
// position) and drawn as they come near the view; far ones are let go again.
import * as pdfjs from "/__condor/pdfjs/pdf.min.mjs";

pdfjs.GlobalWorkerOptions.workerSrc = "/__condor/pdfjs/pdf.worker.min.mjs";

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

async function draw(p) {
  if (p.drawn === scale) return;
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
  try {
    await new pdfjs.TextLayer({textContentSource: page.streamTextContent({includeMarkedContent: true, disableNormalization: true}), container: t, viewport: vp}).render();
  } catch (e) {}
  if (p.drawn !== s) return;
  if (p.text) p.text.replaceWith(t); else p.div.append(t);
  p.text = t;
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
  document.body.append(bar);
  layout();
  await contents().catch(() => {});
  addEventListener("resize", () => layout());
  addEventListener("scroll", () => { redraw(); showPage(); }, {passive: true});
  // Back where the reader was (ui/content.js), else at a page asked for (#page=N, from the
  // built-in Word view).
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
