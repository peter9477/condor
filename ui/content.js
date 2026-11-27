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
    // For pages that build themselves after loading (the built-in Word and PDF views):
    // restore again now that the page has its full height, unless the reader has moved
    // since; then go to a bookmark that was waiting for the page.
    again: function () { if (wanted && !armedByUser) apply(wanted); tryGoto(); },
    locate: locate,
    goto: function (b) { pendingGoto = b; tryGoto(); },
  };

  // ----- Bookmarks. locate(): where the reader is, for a new bookmark; goto(): back there.
  // Pages that build themselves set __condorBusy until they call again(); pages with pages
  // of their own (the PDF view) add __condorLocate / __condorGoto.
  var pendingGoto = null;
  function tryGoto() {
    if (!pendingGoto || window.__condorBusy) return;
    var b = pendingGoto;
    pendingGoto = null;
    doGoto(b);
  }
  function viewName() {
    var q = location.search.slice(1).split("&");
    return q.indexOf("docx") >= 0 ? "web" : q.indexOf("converted") >= 0 ? "pdf" : q.indexOf("source") >= 0 ? "source" : "";
  }
  function squash(t) { return String(t || "").replace(/\s+/g, " ").trim(); }
  // Some text at the top of the view, for a bookmark made without a selection.
  function nearText() {
    var ys = [30, 70, 120, 200, 300], xs = [0.3, 0.5, 0.15, 0.7];
    for (var i = 0; i < ys.length; i++) for (var j = 0; j < xs.length; j++) {
      var r = document.caretRangeFromPoint && document.caretRangeFromPoint(innerWidth * xs[j], ys[i]);
      var n = r && r.startContainer;
      if (n && n.nodeType === 3 && !floating(n.parentElement)) {
        var t = squash(n.parentElement ? n.parentElement.innerText || n.textContent : n.textContent);
        if (t.length > 3) return t.slice(0, 200);
      }
    }
    return "";
  }
  // Inside something that floats over the document (a viewer's toolbar, its contents panel).
  function floating(el) {
    for (; el && el !== document.body; el = el.parentElement) {
      var pos = getComputedStyle(el).position;
      if (pos === "fixed" || pos === "sticky") return true;
    }
    return false;
  }
  function locate() {
    var s = window.getSelection();
    var text = s && !s.isCollapsed ? squash(String(s)) : "";
    var el = document.scrollingElement || document.documentElement;
    var top = window.scrollY;
    if (text) { try { top += s.getRangeAt(0).getBoundingClientRect().top; } catch (e) {} }
    var at = { y: Math.round(window.scrollY), top: Math.round(top), h: el.scrollHeight, sel: text ? selection() : null };
    var frac = Math.max(0, Math.min(1, top / Math.max(1, el.scrollHeight)));
    var label = Math.round(frac * 100) + "%";
    var own = window.__condorLocate && window.__condorLocate();
    if (own) {
      for (var k in own) at[k] = own[k];
      if (own.frac != null) frac = own.frac;
      if (own.label) label = own.label;
    }
    return { at: at, text: (text || nearText()).slice(0, 2000), frac: frac, label: label };
  }
  function reveal(r) {
    var box = r.getBoundingClientRect();
    if (box.top < 40 || box.bottom > innerHeight - 20) window.scrollBy(0, box.top - innerHeight / 3);
  }
  function doGoto(b) {
    restoring = false; armed = true; armedByUser = true; wanted = null;
    var at = b.at || {};
    if (window.__condorGoto && window.__condorGoto(b)) { saveSoon(); return; }
    var el = document.scrollingElement || document.documentElement;
    var s = window.getSelection();
    // The same document as when it was made: the same selection.
    if (at.sel && b.text) {
      try {
        var a = nodeAt(at.sel.a), f = nodeAt(at.sel.f);
        if (a && f) {
          s.setBaseAndExtent(a, at.sel.ao, f, at.sel.fo);
          if (squash(String(s)).slice(0, 60) === b.text.slice(0, 60)) { reveal(s.getRangeAt(0)); saveSoon(); return; }
          s.removeAllRanges();
        }
      } catch (e) {}
    }
    // Else about as far through it, and the text nearest there if it can be found.
    var y = b.view === viewName() && at.h === el.scrollHeight && at.top != null ? at.top : (b.frac || 0) * el.scrollHeight;
    if (!(at.sel && b.text && findNear(b.text.slice(0, 120), y))) window.scrollTo(0, Math.max(0, y - innerHeight / 3));
    saveSoon();
  }
  // The match of t nearest y (document px), selected (window.find steps through matches).
  function findNear(t, y) {
    if (!window.find) return false;
    var s = window.getSelection(), best = null, bd = Infinity, n = 0;
    s.removeAllRanges();
    while (n++ < 300 && window.find(t, false, false, false)) {
      var r = s.getRangeAt(0), d = Math.abs(r.getBoundingClientRect().top + window.scrollY - y);
      if (d < bd) { bd = d; best = r.cloneRange(); }
    }
    s.removeAllRanges();
    if (!best) return false;
    s.addRange(best);
    window.scrollTo(0, 0);
    reveal(best);
    return true;
  }

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
  // Left/Right and Ctrl+PgUp/PgDn page back and forward: a page at a time in documents that
  // have pages (they set __condorPage), else a screenful (Left/Right not when the document
  // scrolls sideways).
  window.addEventListener("keydown", function (e) {
    if (e.altKey || e.metaKey || e.shiftKey) return;
    var arrow = !e.ctrlKey && (e.key === "ArrowLeft" || e.key === "ArrowRight");
    var page = e.ctrlKey && (e.key === "PageUp" || e.key === "PageDown");
    if (!arrow && !page) return;
    var t = e.target;
    if (t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
    var dir = e.key === "ArrowRight" || e.key === "PageDown" ? 1 : -1;
    if (window.__condorPage) { e.preventDefault(); window.__condorPage(dir); return; }
    var el = document.scrollingElement || document.documentElement;
    if (arrow && el.scrollWidth > el.clientWidth + 2) return;
    e.preventDefault();
    window.scrollBy({ top: dir * Math.max(40, window.innerHeight - 40), behavior: "instant" });
  }, true);
  // Esc returns the keyboard to the tree (Enter there comes back without reloading).
  window.addEventListener("keydown", function (e) {
    if (e.key !== "Escape" || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    fetch("/__condor/tree").catch(function () {});
  });
  window.addEventListener("scroll", saveSoon, { passive: true });
  document.addEventListener("selectionchange", saveSoon);
  window.addEventListener("pagehide", save);
})();
