// The built-in Word view: renders the .docx (fetched raw from Condor) with docx-preview,
// and scales its pages down to fit a narrow pane.
(async () => {
  const out = document.getElementById("doc");
  const fit = () => {
    const w = out.querySelector(".docx-wrapper");
    const page = w && w.querySelector("section.docx");
    if (!page) return;
    w.style.zoom = "";
    const need = page.offsetWidth + 32;
    w.style.zoom = String(Math.min(1, document.documentElement.clientWidth / need));
  };
  try {
    const r = await fetch(location.pathname + "?raw");
    if (!r.ok) throw new Error(r.status + " " + r.statusText);
    await docx.renderAsync(await r.arrayBuffer(), out, null, {
      className: "docx", inWrapper: true, breakPages: true, ignoreLastRenderedPageBreak: true,
      experimental: true, renderAltChunks: false, useBase64URL: false,
    });
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
