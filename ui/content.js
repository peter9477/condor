// Injected by Condor into every document in the content view (top frame only). It has no
// IPC: it keeps each document's scroll position and text selection in this view's session
// storage, so moving between documents and back puts you where you were. Condor also
// polls `__condor.state()` to keep them across restarts, and calls `__condor.restore()`
// with the saved values when this session has none.
(function () {
  if (window.top !== window || window.__condor) return;
  var key = "condor:" + location.pathname + location.search;  // source and rendered views apart
  // Nothing is saved until a restore has run or the reader has touched the page, so a stray
  // event during loading can't record "top of page" over the saved position.
  var armed = false, restoring = false, timer = null, wanted = null;

  function nodePath(n) {
    var p = [];
    while (n && n !== document.documentElement) {
      var parent = n.parentNode;
      if (!parent) return null;
      p.unshift(Array.prototype.indexOf.call(parent.childNodes, n));
      n = parent;
    }
    return n ? p : null;
  }
  function nodeAt(p) {
    var n = document.documentElement;
    for (var i = 0; n && i < p.length; i++) n = n.childNodes[p[i]];
    return n || null;
  }
  function selection() {
    var s = window.getSelection();
    if (!s || s.rangeCount === 0 || s.isCollapsed) return null;
    var a = nodePath(s.anchorNode), f = nodePath(s.focusNode);
    return a && f ? { a: a, ao: s.anchorOffset, f: f, fo: s.focusOffset } : null;
  }
  function state() {
    return { x: Math.round(window.scrollX), y: Math.round(window.scrollY), sel: selection() };
  }
  function save() {
    if (restoring || !armed) return;
    try { sessionStorage.setItem(key, JSON.stringify(state())); } catch (e) {}
  }
  function saveSoon() {
    if (!timer) timer = setTimeout(function () { timer = null; save(); }, 200);
  }
  var armedByUser = false;
  function userActs() { restoring = false; armed = true; armedByUser = true; }

  function apply(st) {
    if (!st) return;
    wanted = st;
    var x = st.x || 0, y = st.y || 0, n = 0;
    if (st.sel) {
      try {
        var a = nodeAt(st.sel.a), f = nodeAt(st.sel.f);
        if (a && f) window.getSelection().setBaseAndExtent(a, st.sel.ao, f, st.sel.fo);
      } catch (e) {}
    }
    if (x === 0 && y === 0) return;
    // Retry for a while: images and fonts may still be changing the page height.
    restoring = true;
    (function step() {
      if (!restoring) return;
      window.scrollTo(x, y);
      if ((Math.abs(window.scrollY - y) > 2 || Math.abs(window.scrollX - x) > 2) && n++ < 30) setTimeout(step, 100);
      else restoring = false;
    })();
  }

  window.__condor = {
    state: state,
    hasSession: function () { try { return !!sessionStorage.getItem(key); } catch (e) { return false; } },
    restore: function (st) { if (!this.hasSession()) apply(st); armed = true; },
    // For pages that build themselves after loading (the built-in Word view): restore again
    // now that the page has its full height, unless the reader has moved since.
    again: function () { if (wanted && !armedByUser) apply(wanted); },
  };

  function start() {
    var saved = null;
    try { saved = JSON.parse(sessionStorage.getItem(key) || "null"); } catch (e) {}
    if (saved) { apply(saved); armed = true; }
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();

  // The reader taking over ends a restore in progress.
  ["wheel", "mousedown", "keydown", "touchstart"].forEach(function (t) {
    window.addEventListener(t, userActs, { capture: true, passive: true });
  });
  // Ctrl+U switches between the rendered and source views (the sidebar does the same).
  window.addEventListener("keydown", function (e) {
    if (!e.ctrlKey || e.altKey || e.shiftKey || (e.key !== "u" && e.key !== "U")) return;
    if (!/\.(md|markdown|mdown|mkd|mdx|html?|xhtml|svg)$/i.test(location.pathname)) return;
    e.preventDefault();
    var source = /(^|&)source(&|$)/.test(location.search.slice(1));
    location.href = location.pathname + (source ? "" : "?source");
  }, true);
  // Left/Right page back and forward: a page at a time in documents that have pages (they
  // set __condorPage), else a screenful, unless the document scrolls sideways.
  window.addEventListener("keydown", function (e) {
    if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    var t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    var dir = e.key === "ArrowRight" ? 1 : -1;
    if (window.__condorPage) { e.preventDefault(); window.__condorPage(dir); return; }
    var el = document.scrollingElement || document.documentElement;
    if (el.scrollWidth > el.clientWidth + 2) return;
    e.preventDefault();
    window.scrollBy({ top: dir * Math.max(40, window.innerHeight - 40), behavior: "instant" });
  }, true);
  window.addEventListener("scroll", saveSoon, { passive: true });
  document.addEventListener("selectionchange", saveSoon);
  window.addEventListener("pagehide", save);
})();
