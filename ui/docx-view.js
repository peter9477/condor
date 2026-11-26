// The built-in Word view: renders the .docx (fetched raw from Condor) with docx-preview,
// and scales its pages down to fit a narrow pane.
// docx-preview only starts a page at explicit breaks; this flows each page's overflow onto
// new pages (same header and footer), splitting long tables between rows, then numbers them.
function paginate(root) {
  const px = (v) => parseFloat(v) || 0;
  for (let i = 0; i < 500; i++) {
    const pages = [...root.querySelectorAll("section.docx")];
    let moved = false;
    for (const page of pages) {
      const art = page.querySelector(":scope > article");
      if (!art) continue;
      const cs = getComputedStyle(page);
      const limit = page.getBoundingClientRect().top + px(cs.minHeight) - px(cs.paddingBottom) + 1;
      const kids = [...art.children];
      let cut = kids.findIndex((k) => k.getBoundingClientRect().bottom > limit);
      if (cut < 0) continue;
      let carry = null;
      const k = kids[cut];
      // A table that overflows: its rows that fit stay, the rest continue on the next page.
      if (k.tagName === "TABLE") {
        const rows = [...k.querySelectorAll(":scope > tbody > tr, :scope > tr")];
        const r = rows.findIndex((row) => row.getBoundingClientRect().bottom > limit);
        if (r > 0) {
          carry = k.cloneNode(false);
          for (const c of k.children) if (c.tagName !== "TBODY" && c.tagName !== "TR") carry.append(c.cloneNode(true));
          const body = rows[0].parentElement.tagName === "TBODY" ? document.createElement("tbody") : carry;
          if (body !== carry) carry.append(body);
          for (const row of rows.slice(r)) body.append(row);
          cut++;
        }
      }
      // Headings and titles go with what follows them (as Word keeps them).
      if (!carry) {
        // (A short line just before table-of-contents entries is their title.)
        const heading = (e) => e && e.tagName === "P" && (/\bdocx_(heading\d|title|subtitle|tocheading)\b/i.test(e.className) ||
          (e.textContent.trim().length < 60 && /\bdocx_toc\d\b/.test((e.nextElementSibling || {}).className || "")));
        let c = cut;
        while (c > 1 && heading(kids[c - 1])) c--;
        if (c > 0 && c < cut) cut = c;
      }
      // A single block taller than a page stays where it is.
      if (cut === 0 && !carry) cut = 1;
      const rest = kids.slice(cut);
      if (!rest.length && !carry) continue;
      const next = page.cloneNode(false);
      for (const c of page.children) next.append(c === art ? document.createElement("article") : c.cloneNode(true));
      const nart = next.querySelector(":scope > article");
      if (carry) nart.append(carry);
      for (const r of rest) nart.append(r);
      page.after(next);
      moved = true;
      break;
    }
    if (!moved) break;
  }
  // Drop pages left with nothing on them; then number the pages.
  for (const page of root.querySelectorAll("section.docx")) {
    const art = page.querySelector(":scope > article");
    if (art && !art.textContent.trim() && !art.querySelector("img, svg, table")) page.remove();
  }
  root.querySelectorAll("section.docx").forEach((page, i) => {
    for (const n of page.querySelectorAll(".docx_pagenumber")) n.textContent = String(i + 1);
  });
}

// The contents menu, in a panel that stays open beside the pages and marks the current
// section: the document's own table of contents (its entries link to the sections), else its
// headings (Heading 1-9), numbered as the document numbers them.
function fromToc(root) {
  const out = [];
  for (const p of root.querySelectorAll("article p[class*='docx_toc']")) {
    const level = +(p.className.match(/\bdocx_toc(\d)\b/) || [])[1];
    const a = p.querySelector("a[href^='#']");
    const target = a && document.getElementById(decodeURIComponent(a.getAttribute("href").slice(1)));
    const e = target && (target.closest("p") || target);
    if (!level || !e) continue;
    const first = (a.firstElementChild || {}).textContent || "";
    const num = /^\d+(\.\d+)*\.?$/.test(first.trim()) ? first.trim().replace(/\.$/, "") : "";
    let text = e.textContent.replace(/\s+/g, " ").trim();
    if (num && text.startsWith(num + " ")) text = text.slice(num.length + 1);
    if (text) out.push({e, level, text, num, fixed: true});
  }
  return out;
}
function contents(root) {
  let hs = fromToc(root);
  if (hs.length < 2) hs = [...root.querySelectorAll("article p[class*='docx_heading']")]
    .map((e) => ({e, level: +(e.className.match(/\bdocx_heading(\d)\b/) || [])[1], text: e.textContent.replace(/\s+/g, " ").trim()}))
    .filter((h) => h.level && h.text);
  if (hs.length < 2) return;
  const counts = [];
  for (const h of hs) {
    if (h.fixed || !/\bdocx-num-/.test(h.e.className)) continue;
    counts.length = h.level;
    for (let i = 0; i < h.level - 1; i++) counts[i] = counts[i] || 0;
    counts[h.level - 1] = (counts[h.level - 1] || 0) + 1;
    h.num = counts.slice(0, h.level).join(".");
  }
  const top = Math.min(...hs.map((h) => h.level));
  const btn = document.createElement("button");
  btn.id = "toc-btn";
  btn.title = "Contents";
  btn.textContent = "\u2630";
  const panel = document.createElement("nav");
  panel.id = "toc";
  panel.hidden = true;
  const head = document.createElement("div");
  head.className = "toc-head";
  head.textContent = "Contents";
  const close = document.createElement("button");
  close.textContent = "\u00d7";
  close.title = "Close (Esc)";
  head.append(close);
  panel.append(head);
  const items = hs.map((h) => {
    const a = document.createElement("a");
    a.style.paddingLeft = 10 + 14 * (h.level - top) + "px";
    a.className = "l" + (h.level - top);
    if (h.num) {
      const n = document.createElement("span");
      n.className = "num";
      n.textContent = h.num;
      a.append(n);
    }
    a.append(h.text);
    a.onclick = (e) => { e.preventDefault(); h.e.scrollIntoView({block: "start"}); };
    panel.append(a);
    return a;
  });
  const show = (on) => {
    panel.hidden = !on;
    document.body.classList.toggle("toc-open", on);
    btn.classList.toggle("on", on);
    try { localStorage.setItem("condor-docx-toc", on ? "1" : "0"); } catch (e) {}
    if (on) mark();
    dispatchEvent(new Event("resize"));
  };
  let current = -1;
  const mark = () => {
    if (panel.hidden) return;
    let i = 0;
    hs.forEach((h, j) => { if (h.e.getBoundingClientRect().top < 12) i = j; });
    if (i === current) return;
    if (current >= 0) items[current].classList.remove("cur");
    items[i].classList.add("cur");
    current = i;
    items[i].scrollIntoView({block: "nearest"});
  };
  btn.onclick = () => show(panel.hidden);
  close.onclick = () => show(false);
  addEventListener("keydown", (e) => { if (e.key === "Escape" && !panel.hidden) show(false); });
  addEventListener("scroll", mark, {passive: true});
  document.body.append(panel, btn);
  let saved = null;
  try { saved = localStorage.getItem("condor-docx-toc"); } catch (e) {}
  if (saved === "1") show(true);
}

(async () => {
  const out = document.getElementById("doc");
  const fit = () => {
    const w = out.querySelector(".docx-wrapper");
    const page = w && w.querySelector("section.docx");
    if (!page) return;
    w.style.zoom = "";
    const need = page.offsetWidth + 32;
    const toc = document.getElementById("toc");
    const room = document.documentElement.clientWidth - (toc && !toc.hidden ? toc.offsetWidth : 0);
    w.style.zoom = String(Math.min(1, room / need));
  };
  try {
    const r = await fetch(location.pathname + "?raw");
    if (!r.ok) throw new Error(r.status + " " + r.statusText);
    let data = await r.arrayBuffer();
    // Windows metafile pictures (EMF/WMF), which browsers can't show, are swapped for PNGs
    // that Condor draws from them.
    const zip = await JSZip.loadAsync(data);
    const metas = Object.keys(zip.files).filter((n) => /\.(emf|wmf)$/i.test(n));
    if (metas.length) {
      await Promise.all(metas.map(async (n) => {
        const m = await fetch(location.pathname + "?media=" + encodeURIComponent(n));
        if (m.ok) zip.file(n, await m.arrayBuffer());
      }));
      data = await zip.generateAsync({type: "uint8array"});
    }
    await docx.renderAsync(data, out, null, {
      className: "docx", inWrapper: true, breakPages: true, ignoreLastRenderedPageBreak: true,
      experimental: true, renderAltChunks: false, useBase64URL: false,
    });
    paginate(out);
    contents(out);
    fit();
    addEventListener("resize", fit);
  } catch (e) {
    out.innerHTML = "";
    const d = document.createElement("div");
    d.className = "msg";
    const h = document.createElement("h2");
    h.textContent = "Can't show this document here";
    const p = document.createElement("p");
    p.textContent = (e && e.message ? e.message + ". " : "") + "Try PDF or Windows in the bar above.";
    d.append(h, p);
    out.append(d);
  }
})();
