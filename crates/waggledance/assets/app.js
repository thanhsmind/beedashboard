// Theme toggle (cycles light → dark) with persistence, and WebSocket live reload.
(function () {
  "use strict";

  // review-p1-fixes D3: `data-term-base` is markdown-adjacent, attacker-reachable
  // data (a hostile `<pre data-term-base="https://evil.tld/x">` in a rendered
  // file) that every terminal poller/poster below uses as a fetch/postJson URL
  // prefix. The sanitizer (render.rs) now closes the open `data-*` allowlist
  // that let it survive at all — this is the second, independent gate: even if
  // one ever slips through, only a same-origin `/p/<project>/...` shape on this
  // daemon's own origin is accepted. Anything else (an absolute URL, a
  // protocol-relative `//host/...`, a bare path outside `/p/`) resolves to
  // `null`, and every call site below already falls back to its safe
  // `projectId`-built path — exactly as if `data-term-base` were simply absent.
  function validTermBase(base) {
    if (typeof base !== "string" || base === "") return null;
    // Reject a scheme (`https:`) or a protocol-relative prefix (`//host/...`)
    // before ever handing the string to `URL` — both would make `base + "/x"`
    // resolve off-origin the moment a caller fetches it.
    if (base.charAt(0) !== "/" || base.charAt(1) === "/") return null;
    if (!/^\/p\/[^/]+\/.+$/.test(base)) return null;
    try {
      var resolved = new URL(base, window.location.origin);
      if (resolved.origin !== window.location.origin) return null;
    } catch (e) {
      return null;
    }
    return base;
  }

  // unassigned-poller-guard D1: `validTermBase` above already collapses an
  // unusable/hostile `data-term-base` down to `null`; this answers the
  // other half of the question every screen poll and pane post below has
  // to ask first — given that already-validated `base` and the page's own
  // `projectId`, is there ANY usable target at all? A page with neither
  // (the Unassigned page: no `data-project-id` on `<main>`, no
  // `data-term-base` on its panes — it is wired instead by its own scoped
  // IIFE further down, keyed off `data-unassigned-base`) must never fall
  // through to building a `/p/null/...` URL from that pair. Shared by the
  // screen poller and all three posters (input/keys/attach) below so the OR
  // lives in exactly one place, not reimplemented at each call site.
  function hasTarget(base, projectId) {
    return base != null || projectId != null;
  }

  // One-shot storage-key migration (D5 of the waggledance rename): the old
  // "mdview-theme" / "mdview-folders-open" key is read exactly once, copied
  // to its new "waggledance-*" key, then deleted — so neither the user's
  // theme nor their open-folder state is silently lost, and every read
  // after the first is a plain hit on the new key with the old one gone.
  function migrateStorageKey(storage, oldKey, newKey) {
    try {
      if (storage.getItem(newKey) === null) {
        var old = storage.getItem(oldKey);
        if (old !== null) {
          storage.setItem(newKey, old);
          storage.removeItem(oldKey);
        }
      }
    } catch (e) {}
  }

  function applyTheme(t) {
    var dark = t === "dark" || (t === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.setAttribute("data-scheme", dark ? "dark" : "light");
  }

  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    toggle.addEventListener("click", function () {
      var cur = document.documentElement.getAttribute("data-scheme");
      var next = cur === "dark" ? "light" : "dark";
      try { localStorage.setItem("waggledance-theme", next); } catch (e) {}
      applyTheme(next);
      // Re-render mermaid diagrams for the new theme, if present.
      if (window.__mermaid) {
        try {
          window.__mermaid.initialize({ startOnLoad: false, theme: next === "dark" ? "dark" : "default" });
        } catch (e) {}
      }
    });
  }

  // Chapter sidebar (C2 breadcrumb-zoom): always show exactly one folder —
  // its subfolders (zoom in) and its files by title — with a clickable
  // breadcrumb to zoom out. Default focus = the current file's folder.
  (function () {
    var root = document.getElementById("chapter");
    var data = document.getElementById("filelist");
    if (!root || !data) return;

    var files;
    try { files = JSON.parse(data.textContent || "[]"); } catch (e) { return; }
    var pid = root.getAttribute("data-pid") || "";
    var rootLabel = root.getAttribute("data-root") || "/";
    var current = root.getAttribute("data-current") || "";

    function dirOf(p) { var i = p.lastIndexOf("/"); return i < 0 ? "" : p.slice(0, i); }
    function baseOf(p) { var i = p.lastIndexOf("/"); return i < 0 ? p : p.slice(i + 1); }
    function el(tag, cls, text) {
      var e = document.createElement(tag);
      if (cls) e.className = cls;
      if (text != null) e.textContent = text;
      return e;
    }

    var focus = dirOf(current); // start in the current file's folder

    // Whether the subfolders disclosure is expanded — remembered for the
    // session (auto-opens when a folder has no files of its own, see below).
    var foldersOpen = false;
    try {
      migrateStorageKey(sessionStorage, "mdview-folders-open", "waggledance-folders-open");
      foldersOpen = sessionStorage.getItem("waggledance-folders-open") === "1";
    } catch (e) {}

    function render() {
      root.textContent = "";

      // Breadcrumb: root + each ancestor segment, all clickable to zoom out.
      var bc = el("div", "chap-crumbs");
      var rootSeg = el("button", "chap-seg", rootLabel);
      rootSeg.addEventListener("click", function () { focus = ""; render(); });
      bc.appendChild(rootSeg);
      if (focus) {
        var segs = focus.split("/");
        var acc = "";
        segs.forEach(function (s) {
          acc = acc ? acc + "/" + s : s;
          var path = acc;
          bc.appendChild(el("span", "chap-sep", "›"));
          var b = el("button", "chap-seg", s);
          b.addEventListener("click", function () { focus = path; render(); });
          bc.appendChild(b);
        });
      }
      root.appendChild(bc);

      // Partition the focus folder into immediate subfolders and direct files.
      var prefix = focus ? focus + "/" : "";
      var folders = {};
      var here = [];
      files.forEach(function (f) {
        if (focus && f.p.indexOf(prefix) !== 0) return;
        var rest = focus ? f.p.slice(prefix.length) : f.p;
        var slash = rest.indexOf("/");
        if (slash < 0) here.push(f);
        else folders[rest.slice(0, slash)] = true;
      });
      var folderNames = Object.keys(folders).sort();

      // Every subfolder collapses into ONE disclosure bar, so however many there
      // are they never crowd out the chapter list. Collapsed by default; opens
      // automatically when this folder has no files (else it would look empty).
      if (folderNames.length) {
        var open = foldersOpen || here.length === 0;
        var box = el("div", "chap-folders" + (open ? " is-open" : ""));

        var bar = el("button", "chap-folders__bar");
        bar.setAttribute("aria-expanded", open ? "true" : "false");
        bar.appendChild(el("span", "chap-folders__chev", "›"));
        bar.appendChild(el("span", "chap-folders__label", "Subfolders"));
        bar.appendChild(el("span", "chap-folders__count", String(folderNames.length)));
        bar.addEventListener("click", function () {
          foldersOpen = !box.classList.contains("is-open");
          try { sessionStorage.setItem("waggledance-folders-open", foldersOpen ? "1" : "0"); } catch (e) {}
          box.classList.toggle("is-open", foldersOpen);
          bar.setAttribute("aria-expanded", foldersOpen ? "true" : "false");
        });
        box.appendChild(bar);

        var list = el("div", "chap-folders__list");
        var inner = el("div", "chap-folders__inner");
        folderNames.forEach(function (name) {
          var b = el("button", "chap-subfolder", name);
          b.addEventListener("click", function () {
            focus = focus ? focus + "/" + name : name;
            render();
          });
          inner.appendChild(b);
        });
        list.appendChild(inner);
        box.appendChild(list);
        root.appendChild(box);
      }

      // The chapter list: files in this folder, by title, current one active.
      if (here.length) {
        root.appendChild(el("div", "chap-sec", "Chapters"));
        here
          .map(function (f) { return { f: f, label: f.t && f.t.length ? f.t : baseOf(f.p) }; })
          .sort(function (a, b) { return a.label.localeCompare(b.label); })
          .forEach(function (item) {
            var a = el("a", "chap-file" + (item.f.p === current ? " active" : ""), item.label);
            a.href = "/p/" + pid + "/" + item.f.p;
            root.appendChild(a);
          });
      }
    }

    render();
  })();

  // Code section's folder disclosure: code_tree() (views.rs) server-renders
  // the same `.chap-folders` markup the block above builds client-side for
  // Docs, so this only needs to wire the click-to-toggle + remembered-open
  // behavior, not build any DOM. Runs once at load — and the Docs block
  // above already ran its `render()` synchronously, so its bar IS in the DOM
  // by now and already carries its own handler. Attaching a second handler
  // to it would toggle the disclosure twice per click (open then straight
  // back closed), so skip anything inside the Docs sidebar (`#chapter`) —
  // the Code sidebar's `nav.chapter` has no id.
  (function () {
    var docsNav = document.getElementById("chapter");
    var bars = [].slice.call(document.querySelectorAll(".chap-folders__bar"))
      .filter(function (bar) { return !(docsNav && docsNav.contains(bar)); });
    if (!bars.length) return;
    var remembered = false;
    try {
      migrateStorageKey(sessionStorage, "mdview-folders-open", "waggledance-folders-open");
      remembered = sessionStorage.getItem("waggledance-folders-open") === "1";
    } catch (e) {}
    bars.forEach(function (bar) {
      var box = bar.closest(".chap-folders");
      if (!box) return;
      if (remembered) {
        box.classList.add("is-open");
        bar.setAttribute("aria-expanded", "true");
      }
      bar.addEventListener("click", function () {
        var open = !box.classList.contains("is-open");
        box.classList.toggle("is-open", open);
        bar.setAttribute("aria-expanded", open ? "true" : "false");
        try { sessionStorage.setItem("waggledance-folders-open", open ? "1" : "0"); } catch (e) {}
      });
    });
  })();

  // Fuzzy file-jump palette (Cmd/Ctrl+K): fetch nucleo-ranked files from the
  // server /p/:id/_jump endpoint and navigate. Complements full-text search —
  // this jumps by file name/path, that searches content.
  (function () {
    var chapter = document.getElementById("chapter");
    var pid = chapter && chapter.getAttribute("data-pid");
    if (!pid) return;

    var overlay, input, list;
    var hits = [];
    var sel = 0;
    var seq = 0; // request sequence — drop responses that a later query superseded
    var timer = null;

    function build() {
      overlay = document.createElement("div");
      overlay.className = "jump-overlay";
      overlay.setAttribute("hidden", "");
      var box = document.createElement("div");
      box.className = "jump-box";
      input = document.createElement("input");
      input.className = "jump-input";
      input.type = "text";
      input.placeholder = "Jump to file…";
      input.setAttribute("aria-label", "Jump to file");
      list = document.createElement("ul");
      list.className = "jump-list";
      box.appendChild(input);
      box.appendChild(list);
      overlay.appendChild(box);
      document.body.appendChild(overlay);

      overlay.addEventListener("mousedown", function (e) {
        if (e.target === overlay) close();
      });
      input.addEventListener("input", onInput);
      input.addEventListener("keydown", onKey);
    }

    function isOpen() { return overlay && !overlay.hasAttribute("hidden"); }

    function open() {
      if (!overlay) build();
      overlay.removeAttribute("hidden");
      input.value = "";
      hits = [];
      sel = 0;
      render();
      input.focus();
    }

    function close() {
      if (overlay) overlay.setAttribute("hidden", "");
    }

    function onInput() {
      if (timer) clearTimeout(timer);
      timer = setTimeout(fetchHits, 120);
    }

    function fetchHits() {
      var q = input.value.trim();
      if (!q) { hits = []; sel = 0; render(); return; }
      var mine = ++seq;
      fetch("/p/" + encodeURIComponent(pid) + "/_jump?q=" + encodeURIComponent(q))
        .then(function (r) { return r.ok ? r.json() : []; })
        .then(function (data) {
          if (mine !== seq) return; // a newer keystroke already fired
          hits = Array.isArray(data) ? data : [];
          sel = 0;
          render();
        })
        .catch(function () { if (mine === seq) { hits = []; render(); } });
    }

    function render() {
      list.textContent = "";
      hits.forEach(function (h, i) {
        var li = document.createElement("li");
        li.className = "jump-item" + (i === sel ? " active" : "");
        var t = document.createElement("span");
        t.className = "jump-title";
        t.textContent = h.title && h.title.length ? h.title : h.rel_path;
        var p = document.createElement("span");
        p.className = "jump-path";
        p.textContent = h.rel_path;
        li.appendChild(t);
        li.appendChild(p);
        li.addEventListener("mousedown", function (e) { e.preventDefault(); go(i); });
        list.appendChild(li);
      });
    }

    function go(i) {
      var h = hits[i];
      if (h) window.location.href = h.url;
    }

    function onKey(e) {
      if (e.key === "Escape") { e.preventDefault(); close(); }
      else if (e.key === "ArrowDown") { e.preventDefault(); if (hits.length) { sel = (sel + 1) % hits.length; render(); } }
      else if (e.key === "ArrowUp") { e.preventDefault(); if (hits.length) { sel = (sel - 1 + hits.length) % hits.length; render(); } }
      else if (e.key === "Enter") { e.preventDefault(); go(sel); }
    }

    document.addEventListener("keydown", function (e) {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        if (isOpen()) close(); else open();
      }
    });
  })();

  // Copy-as-markdown: when the user copies a selection inside the rendered
  // article, substitute the RAW markdown for the selected block range (mapped
  // via data-sourcepos line numbers) instead of the rendered HTML/text.
  (function () {
    var article = document.querySelector(".markdown-body");
    var srcEl = document.getElementById("mdsource");
    if (!article || !srcEl) return;

    var source;
    try { source = JSON.parse(srcEl.textContent || '""'); } catch (e) { return; }
    if (typeof source !== "string" || !source.length) return;
    var lines = source.split("\n");

    // Parse comrak's data-sourcepos "startLine:col-endLine:col" → [start, end].
    function rangeOf(el) {
      var sp = el.getAttribute("data-sourcepos");
      if (!sp) return null;
      var m = /^(\d+):\d+-(\d+):\d+$/.exec(sp);
      if (!m) return null;
      return [parseInt(m[1], 10), parseInt(m[2], 10)];
    }

    document.addEventListener("copy", function (e) {
      var sel = window.getSelection();
      if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return;

      // Only act when the selection lives inside the rendered article.
      var anchor = sel.anchorNode;
      if (!anchor || !article.contains(anchor)) return;

      // Collect the source line range across every mapped block the selection
      // touches (partial containment), then union to a single [min, max].
      var blocks = article.querySelectorAll("[data-sourcepos]");
      var min = Infinity, max = -Infinity;
      for (var i = 0; i < blocks.length; i++) {
        if (!sel.containsNode(blocks[i], true)) continue;
        var r = rangeOf(blocks[i]);
        if (!r) continue;
        if (r[0] < min) min = r[0];
        if (r[1] > max) max = r[1];
      }
      if (min === Infinity || max < min) return; // nothing mapped → default copy

      var md = lines.slice(min - 1, max).join("\n");
      if (!md) return;
      if (e.clipboardData) {
        e.clipboardData.setData("text/plain", md);
        e.preventDefault();
      }
    });
  })();

  // Project row timestamps: the server sends the full ISO instant in
  // <time datetime> and already prints a minute-precision fallback as the
  // element's text; this upgrades it to the viewer's own locale/timezone —
  // a short relative age while it is recent, an absolute date and time once
  // it is older than a week. Never seconds, never sub-second digits.
  (function () {
    var times = document.querySelectorAll("time.proj-row__time[datetime]");
    if (!times.length) return;
    function fmt(iso) {
      var d = new Date(iso);
      if (isNaN(d.getTime())) return null;
      var secs = (Date.now() - d.getTime()) / 1000;
      if (secs < 60) return "just now";
      if (secs < 3600) return Math.floor(secs / 60) + " min ago";
      if (secs < 86400) return Math.floor(secs / 3600) + "h ago";
      if (secs < 604800) return Math.floor(secs / 86400) + "d ago";
      return d.toLocaleString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    }
    times.forEach(function (t) {
      var iso = t.getAttribute("datetime");
      var s = fmt(iso);
      if (!s) return;
      t.textContent = s;
      t.title = new Date(iso).toLocaleString();
    });
  })();

  // Project delete (home page): confirm before unregistering. The form still
  // POSTs normally if scripting is off — this only guards against a stray tap.
  (function () {
    var forms = document.querySelectorAll(".proj-row__delete");
    if (!forms.length) return;
    forms.forEach(function (f) {
      f.addEventListener("submit", function (e) {
        var name = f.getAttribute("data-project") || "this project";
        var ok = window.confirm(
          "Remove “" + name + "” from waggledance?\n\n" +
          "The files stay on disk — only the registry entry and its index are removed. " +
          "Re-registering re-scans them."
        );
        if (!ok) e.preventDefault();
      });
    });
  })();

  // Project rail filter (home page, console-theme-kanban ctk-12): the
  // rail's search field ships `hidden` from the server. Filtering a list is
  // a client-side act and this page has no server route for it, so a field
  // that promised a search scripting could not deliver would be a control
  // that lies — it is unhidden here, and only here, once the filter is
  // actually wired. Rows whose text does not contain what was typed are
  // hidden; a worktree child stays visible whenever its own parent matched,
  // so a project group never breaks apart mid-filter.
  (function () {
    var box = document.querySelector("[data-proj-filter]");
    if (!box) return;
    var input = box.querySelector(".home-sidebar__filter");
    var rows = document.querySelectorAll(".home-sidebar .proj-row");
    if (!input || !rows.length) return;
    box.hidden = false;
    function apply() {
      var q = input.value.trim().toLowerCase();
      var groupShown = false;
      rows.forEach(function (row) {
        var hit = !q || (row.textContent || "").toLowerCase().indexOf(q) !== -1;
        if (row.classList.contains("proj-row--branch")) {
          if (groupShown) hit = true;
        } else {
          groupShown = hit;
        }
        row.hidden = !hit;
      });
    }
    input.addEventListener("input", apply);
    // Escape clears the field even where the browser draws no clear button.
    input.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && input.value !== "") {
        input.value = "";
        apply();
      }
    });
    apply();
  })();

  // Terminal settings (Settings page, `/api/terminal-config`): D10 requires
  // a JSON body — a plain form POST is a CORS *simple* request (no
  // preflight, no CORS layer on this server), so a page the owner happens
  // to have open could otherwise flip the switches or overwrite the notify
  // credential cross-site using the owner's own Cloudflare Access cookie.
  // JSON forces a preflight this server never answers, closing that gap. A
  // plain HTML `<form>` cannot send JSON, so this intercepts the submit and
  // sends the same field values via `fetch` instead — the controls and the
  // redirect the page lands on are otherwise unchanged from the form this
  // replaces.
  (function () {
    var form = document.getElementById("terminal-config-form");
    if (!form) return;
    form.addEventListener("submit", function (e) {
      e.preventDefault();
      var body = {
        enabled: form.enabled.checked,
        supervisor_enabled: form.supervisor_enabled.checked,
        notify_enabled: form.notify_enabled.checked,
        unassigned_enabled: form.unassigned_enabled.checked,
        notify_chat_id: form.notify_chat_id.value,
        notify_telegram_token: form.notify_telegram_token.value,
      };
      fetch(form.action, {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      })
        .then(function (res) {
          window.location.href = res.url || "/settings";
        })
        .catch(function () {
          window.location.href = "/settings";
        });
    });
  })();

  // Mobile sidebar drawer: the file-tree sidebar is hidden at narrow widths,
  // so the topbar hamburger toggles it open as an overlay (with a backdrop).
  (function () {
    var layout = document.querySelector(".layout");
    var toggle = document.getElementById("sidebar-toggle");
    if (!layout || !toggle) return;
    var backdrop = layout.querySelector(".sidebar-backdrop");
    function set(open) {
      layout.classList.toggle("sidebar-open", open);
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
    }
    toggle.addEventListener("click", function () {
      set(!layout.classList.contains("sidebar-open"));
    });
    if (backdrop) backdrop.addEventListener("click", function () { set(false); });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape") set(false);
    });
    // Picking a file navigates (full reload); a folder click only zooms the
    // tree in place, so close only when an actual file link is chosen.
    var sb = layout.querySelector(".sidebar");
    if (sb) sb.addEventListener("click", function (e) {
      if (e.target.closest(".chap-file")) set(false);
    });
  })();

  // Code-block copy button. Each rendered code block (`<pre class="code">`, not
  // mermaid) gets wrapped in the design system's .fg-codeblock component with a
  // top bar carrying its language label and a Copy button. Done client-side
  // because the server's HTML sanitizer would strip a server-emitted <button>.
  (function () {
    var blocks = document.querySelectorAll(".fg-prose pre.code");
    if (!blocks.length || !document.body) return;

    function langOf(pre) {
      var code = pre.querySelector("code");
      if (!code) return "";
      var m = /(?:^|\s)language-([\w+#.-]+)/.exec(code.className || "");
      return m ? m[1] : "";
    }

    function copyText(text, btn) {
      function ok() {
        var prev = btn.textContent;
        btn.textContent = "Copied";
        btn.classList.add("is-copied");
        setTimeout(function () {
          btn.textContent = prev;
          btn.classList.remove("is-copied");
        }, 1400);
      }
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(ok, function () { fallback(text, ok); });
      } else {
        fallback(text, ok);
      }
    }
    function fallback(text, ok) {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand("copy"); ok(); } catch (e) {}
      document.body.removeChild(ta);
    }

    blocks.forEach(function (pre) {
      if (pre.parentElement && pre.parentElement.classList.contains("fg-codeblock")) return;
      var code = pre.querySelector("code");
      if (!code) return;

      var wrap = document.createElement("div");
      wrap.className = "fg-codeblock";
      var bar = document.createElement("div");
      bar.className = "fg-codeblock__bar";
      var label = document.createElement("span");
      label.className = "fg-codeblock__lang";
      label.textContent = langOf(pre) || "text";
      var btn = document.createElement("button");
      btn.type = "button";
      btn.className = "fg-codeblock__copy";
      btn.textContent = "Copy";
      btn.setAttribute("aria-label", "Copy code to clipboard");
      btn.addEventListener("click", function () { copyText(code.textContent, btn); });
      bar.appendChild(label);
      bar.appendChild(btn);

      pre.parentNode.insertBefore(wrap, pre);
      wrap.appendChild(bar);
      wrap.appendChild(pre);
    });
  })();

  // Copy the whole page's Markdown source (embedded as JSON in #mdsource) —
  // complements the selection-based copy-as-markdown above.
  (function () {
    var btn = document.getElementById("copy-md");
    var src = document.getElementById("mdsource");
    if (!btn || !src) return;
    var md;
    try { md = JSON.parse(src.textContent || '""'); } catch (e) { md = src.textContent || ""; }
    var txt = btn.querySelector(".copy-md__txt");
    function ok() {
      btn.classList.add("is-copied");
      var prev = txt ? txt.textContent : null;
      if (txt) txt.textContent = "Copied";
      setTimeout(function () {
        btn.classList.remove("is-copied");
        if (txt && prev != null) txt.textContent = prev;
      }, 1400);
    }
    function fallback() {
      var ta = document.createElement("textarea");
      ta.value = md;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try { document.execCommand("copy"); ok(); } catch (e) {}
      document.body.removeChild(ta);
    }
    btn.addEventListener("click", function () {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(md).then(ok, fallback);
      } else {
        fallback();
      }
    });
  })();

  // Mermaid pan/zoom + fullscreen. Mermaid renders <pre class="mermaid"> into an
  // <svg> asynchronously (client-side, CDN); we watch for that SVG and wrap each
  // diagram with wheel-zoom / drag-pan / fullscreen — no extra library.
  (function () {
    if (!document.querySelector("pre.mermaid")) return;

    function enhance(pre) {
      if (!pre.querySelector("svg")) return;
      // Controls must live OUTSIDE the <pre>: mermaid overwrites pre.innerHTML
      // (sometimes more than once), which wipes anything appended inside it. So
      // wrap the pre and hang the toolbar on the wrapper instead. The wrapper's
      // presence is also the idempotency guard.
      if (pre.parentElement && pre.parentElement.classList.contains("mermaid-wrap")) return;
      var wrap = document.createElement("div");
      wrap.className = "mermaid-wrap";
      pre.parentNode.insertBefore(wrap, pre);
      wrap.appendChild(pre);
      pre.classList.add("zoomable");
      pre.setAttribute("tabindex", "0");

      var state = { scale: 1, x: 0, y: 0 };
      function apply() {
        // Query the svg fresh — mermaid may replace it (e.g. on theme change).
        var s = pre.querySelector("svg");
        if (s) {
          s.style.transform =
            "translate(" + state.x + "px," + state.y + "px) scale(" + state.scale + ")";
        }
      }
      function clampScale(s) { return Math.min(8, Math.max(0.2, s)); }

      // Zoom toward a point (px,py) in the pre's local coordinates.
      function zoomAt(factor, px, py) {
        var next = clampScale(state.scale * factor);
        var ratio = next / state.scale;
        state.x = px - ratio * (px - state.x);
        state.y = py - ratio * (py - state.y);
        state.scale = next;
        apply();
      }
      // Center the diagram at scale 1 within the current pre box (works for
      // both the normal column and fullscreen, where the pre fills the
      // viewport). Centering lives in the transform so it stays consistent with
      // the zoom/pan math (transform-origin is the svg's top-left).
      function fit() {
        var s = pre.querySelector("svg");
        if (!s) return;
        state.scale = 1;
        state.x = 0;
        state.y = 0;
        s.style.transform = "";
        var sr = s.getBoundingClientRect();
        state.x = Math.max(0, (pre.clientWidth - sr.width) / 2);
        state.y = Math.max(0, (pre.clientHeight - sr.height) / 2);
        apply();
      }
      function reset() { fit(); }

      pre.addEventListener("wheel", function (e) {
        e.preventDefault();
        var rect = pre.getBoundingClientRect();
        zoomAt(e.deltaY < 0 ? 1.15 : 1 / 1.15, e.clientX - rect.left, e.clientY - rect.top);
      }, { passive: false });

      // Drag to pan.
      var dragging = false, sx = 0, sy = 0, ox = 0, oy = 0;
      pre.addEventListener("mousedown", function (e) {
        if (e.target.closest(".mermaid-controls")) return;
        dragging = true; sx = e.clientX; sy = e.clientY; ox = state.x; oy = state.y;
        pre.classList.add("grabbing");
        e.preventDefault();
      });
      window.addEventListener("mousemove", function (e) {
        if (!dragging) return;
        state.x = ox + (e.clientX - sx);
        state.y = oy + (e.clientY - sy);
        apply();
      });
      window.addEventListener("mouseup", function () {
        dragging = false; pre.classList.remove("grabbing");
      });

      // Touch: one-finger pan, two-finger pinch-zoom. Mobile has no hover, so
      // the controls stay visible there (CSS @media (hover: none)) and these
      // gestures make the diagram actually navigable once zoomed.
      var tPan = false, tsx = 0, tsy = 0, tox = 0, toy = 0, pinch = 0;
      function touchDist(t) {
        return Math.hypot(t[0].clientX - t[1].clientX, t[0].clientY - t[1].clientY);
      }
      pre.addEventListener("touchstart", function (e) {
        if (e.target.closest(".mermaid-controls")) return;
        if (e.touches.length === 1) {
          tPan = true; tsx = e.touches[0].clientX; tsy = e.touches[0].clientY;
          tox = state.x; toy = state.y;
        } else if (e.touches.length === 2) {
          tPan = false; pinch = touchDist(e.touches);
        }
      }, { passive: true });
      pre.addEventListener("touchmove", function (e) {
        if (e.touches.length === 2 && pinch > 0) {
          e.preventDefault();
          var rect = pre.getBoundingClientRect();
          var cx = (e.touches[0].clientX + e.touches[1].clientX) / 2 - rect.left;
          var cy = (e.touches[0].clientY + e.touches[1].clientY) / 2 - rect.top;
          var d = touchDist(e.touches);
          zoomAt(d / pinch, cx, cy);
          pinch = d;
        } else if (tPan && e.touches.length === 1) {
          e.preventDefault();
          state.x = tox + (e.touches[0].clientX - tsx);
          state.y = toy + (e.touches[0].clientY - tsy);
          apply();
        }
      }, { passive: false });
      pre.addEventListener("touchend", function (e) {
        if (e.touches.length === 0) { tPan = false; pinch = 0; }
        else if (e.touches.length === 1) {
          // A pinch dropped to one finger → resume panning from here.
          tPan = true; tsx = e.touches[0].clientX; tsy = e.touches[0].clientY;
          tox = state.x; toy = state.y; pinch = 0;
        }
      }, { passive: true });

      // Controls toolbar.
      var controls = document.createElement("div");
      controls.className = "mermaid-controls";
      function btn(label, title, onClick) {
        var b = document.createElement("button");
        b.type = "button";
        b.textContent = label;
        b.title = title;
        b.setAttribute("aria-label", title);
        b.addEventListener("click", function (e) { e.stopPropagation(); onClick(); });
        controls.appendChild(b);
      }
      btn("+", "Zoom in", function () {
        var r = pre.getBoundingClientRect(); zoomAt(1.2, r.width / 2, r.height / 2);
      });
      btn("−", "Zoom out", function () {
        var r = pre.getBoundingClientRect(); zoomAt(1 / 1.2, r.width / 2, r.height / 2);
      });
      btn("⟲", "Reset", reset);
      btn("⛶", "Fullscreen", function () {
        // Fullscreen the wrapper so the toolbar stays visible in fullscreen.
        if (document.fullscreenElement === wrap) {
          if (document.exitFullscreen) document.exitFullscreen();
        } else if (wrap.requestFullscreen) {
          wrap.requestFullscreen();
        }
      });
      wrap.appendChild(controls);

      // Center on first paint and whenever fullscreen toggles (the pre box
      // changes size). rAF so layout has settled before we measure.
      fit();
      requestAnimationFrame(fit);
      document.addEventListener("fullscreenchange", function () {
        requestAnimationFrame(fit);
      });
    }

    // Enhance every diagram that already has its <svg>. Idempotent (enhance
    // no-ops on an already-zoomable pre), so it is safe to call repeatedly.
    function enhanceAll() {
      document.querySelectorAll("pre.mermaid").forEach(function (p) {
        // Isolate failures: one diagram erroring must not block the others.
        try { enhance(p); } catch (e) { console.error("mermaid enhance:", e); }
      });
    }

    // mermaid renders asynchronously. We attach the toolbar through three
    // independent triggers so a missed one never leaves a diagram uncontrolled:
    //   1. the explicit "done" event the page fires after mermaid.run() resolves,
    //   2. a DOM observer catching the injected <svg>,
    //   3. timed sweeps as a final backstop.
    document.addEventListener("waggledance:mermaid-done", enhanceAll);
    var obs = new MutationObserver(enhanceAll);
    obs.observe(document.body, { childList: true, subtree: true });
    [200, 800, 2000, 4000].forEach(function (t) { setTimeout(enhanceAll, t); });
    enhanceAll();
  })();

  // TOC scrollspy: highlight the "On this page" link matching the heading
  // currently in view while the reader scrolls a file page.
  (function () {
    var toc = document.querySelector(".toc");
    var article = document.querySelector(".fg-prose");
    if (!toc || !article) return;

    var links = Array.prototype.slice.call(toc.querySelectorAll("a[href^='#']"));
    if (!links.length) return;

    var linkByHash = {};
    links.forEach(function (a) { linkByHash[a.getAttribute("href")] = a; });

    var headings = links
      .map(function (a) { return document.getElementById(a.getAttribute("href").slice(1)); })
      .filter(Boolean);
    if (!headings.length) return;

    var current = null;
    function setActive(hash) {
      if (hash === current) return;
      if (current && linkByHash[current]) linkByHash[current].classList.remove("active");
      current = hash;
      if (current && linkByHash[current]) linkByHash[current].classList.add("active");
    }

    var observer = new IntersectionObserver(
      function (entries) {
        var visible = entries.filter(function (e) { return e.isIntersecting; });
        if (!visible.length) return;
        // Highest-on-page visible heading wins.
        visible.sort(function (a, b) { return a.boundingClientRect.top - b.boundingClientRect.top; });
        setActive("#" + visible[0].target.id);
      },
      { rootMargin: "0px 0px -70% 0px", threshold: 0 }
    );
    headings.forEach(function (h) { observer.observe(h); });
  })();

  // Live reload, targeted (PRD FR-19): the watcher broadcasts
  // {"changed":["<project_id>/<rel_path>", ...]}. A file page reloads only
  // when its own document is in the list; project-scoped pages (home,
  // search, bee board) reload on any change within their project because
  // the tree and backlinks they render shift with every edit; the root
  // index reloads on any change. Terminal/transcript pages never reload
  // from a markdown edit — they poll their own endpoints and a forced
  // reload would drop in-progress input.
  //
  // homepage-terminal-refresh: that last rule used to be spelled as a path
  // test, and the path test only covered `/p/<id>/_terminal...`. The homepage
  // grew a Terminals tab of its own at `/?tab=terminals`, whose path is `/` —
  // it fell through to the `return true` below and reloaded on any markdown
  // edit in any project, resetting a live terminal mid-session. The honest
  // rule is the reason itself, not the path: a document showing a live screen
  // never force-reloads, wherever it is served from. The Kanban and Projects
  // tabs carry no `.term-screen` and keep reloading exactly as before.
  // home-board-perf: the root index only renders `docs/history/<feature>/
  // CONTEXT.md` markdown (via read_snapshot); a changed entry elsewhere in a
  // project's tree cannot affect what `/` shows, so the home/no-match branch
  // below reloads only when at least one changed entry is board-relevant.
  function isBoardRelevant(changedEntry) {
    var slash = changedEntry.indexOf("/");
    if (slash === -1) return false;
    return changedEntry.slice(slash + 1).indexOf("docs/history/") === 0;
  }
  function shouldReload(changed) {
    if (document.querySelector(".term-screen")) return false;
    var m = location.pathname.match(/^\/p\/([^\/]+)\/(.*)$/);
    if (!m) return changed.some(isBoardRelevant);
    var pid, rest;
    try {
      pid = decodeURIComponent(m[1]);
      rest = decodeURIComponent(m[2]);
    } catch (e) {
      return true;
    }
    if (/^_(terminal|transcript)(\/|$)/.test(rest)) return false;
    if (!rest || rest.charAt(0) === "_") {
      return changed.some(function (c) { return c.indexOf(pid + "/") === 0; });
    }
    return changed.indexOf(pid + "/" + rest) !== -1;
  }
  function connect() {
    var proto = location.protocol === "https:" ? "wss:" : "ws:";
    var ws = new WebSocket(proto + "//" + location.host + "/ws");
    ws.onmessage = function (ev) {
      if (ev.data === "reload") { location.reload(); return; }
      var msg;
      try { msg = JSON.parse(ev.data); } catch (e) { return; }
      if (msg && Array.isArray(msg.changed) && shouldReload(msg.changed)) {
        location.reload();
      }
    };
    ws.onclose = function () { setTimeout(connect, 3000); };
    ws.onerror = function () { try { ws.close(); } catch (e) {} };
  }
  connect();

  // Terminal screen poll (agent-terminal-6, ANSI rendering agent-terminal-12):
  // each pane's `.term-screen` viewport polls its own
  // `/p/:id/_terminal/:pane_id/screen` endpoint on a fixed interval. The
  // server (`waggledance_core::ansi::to_html`) has already translated herdr's raw
  // ANSI screen into safe, escaped HTML carrying `ansi-*` colour/attribute
  // classes — never xterm.js, this is a polled snapshot, not a live PTY — so
  // the poller assigns it via `innerHTML`, not `textContent`. A `revision`
  // that hasn't changed since the last successful poll skips the repaint.
  //
  // On any failed poll (herdr silent, the pane gone, the network hiccups)
  // the viewport shows the same "herdr is not running" wording the page's
  // own down-state renders (D6) — never left blank, and never mistaken for
  // "the pane has no output". The interval itself never changes on failure:
  // there is no backoff and no faster retry, so a herdr outage can never
  // turn this poller into a request storm against a socket that is already
  // struggling to answer.
  (function () {
    // homepage-terminals: the home page's own Terminals tab has no single
    // `data-project-id` to bootstrap from — its panes can belong to any
    // project or to none (D3) — so `main` (and therefore `projectId`) is
    // allowed to be absent here; each `.term-screen` instead carries its
    // own `data-term-base` (`views.rs::screen_frame`), read per-element in
    // `pollOne` below. The project and Unassigned pages are untouched:
    // neither renders `data-term-base`, so both keep resolving through
    // `projectId` exactly as before.
    var main = document.querySelector("main.fg-page[data-project-id]");
    var projectId = main ? main.getAttribute("data-project-id") : null;
    var screens = Array.prototype.slice.call(document.querySelectorAll(".term-screen[data-pane-id]"));
    if (!screens.length) return;

    var POLL_MS = 1500;
    var HERDR_DOWN_TEXT = "herdr is not running";
    var lastRevision = {}; // pane id -> last-rendered revision
    // poller-inflight-guard D1: mirrors the transcript poller's own
    // `inFlight` map below — a pane whose screen fetch is still outstanding
    // (each fetch holds the server-side per-pane `pane_lock`) is skipped on
    // the next tick rather than stacking a second request behind it.
    var inFlightScreen = {}; // pane id -> a screen fetch for this pane is still outstanding
    // terminal-scroll-2: true while this pane's "Load older" button owns the
    // viewport — pollOne below must never overwrite that history view with
    // the next live-tail repaint before the operator has looked at it and
    // pressed "Live" (or clicked "Load older" again). Cleared only by the
    // "Live" button's handler further down.
    var viewingHistory = {}; // pane id -> true while showing history, not live
    // scroll-keep-position review fix (C): each pane's own last-requested
    // absolute depth, kept at this outer scope (not just the per-pane
    // closure local it used to be) so the best-effort pagehide/
    // visibilitychange restore below can see every scrolled pane, not only
    // the one whose button was clicked most recently.
    var paneHistoryDepth = {}; // pane id -> 0 = live; last absolute depth requested

    // homepage-terminals: `base`, when given, is a `.term-screen` element's
    // own `data-term-base` (already the pane's full route prefix, pane id
    // included — `views.rs::screen_frame`'s doc) and wins outright; omitted
    // (every call this feature did not touch — the scroll-history buttons
    // and the pagehide/visibilitychange restore further down, none of
    // which run on the home page since it renders no `.term-scroll`),
    // `screenUrl` falls back to the project page's own `projectId`-built
    // path exactly as before.
    function screenUrl(paneId, historyDepth, base) {
      var url = base
        ? base + "/screen"
        : "/p/" + encodeURIComponent(projectId) + "/_terminal/" + encodeURIComponent(paneId) + "/screen";
      // scroll-keep-position: `historyDepth` is now the pane's absolute
      // requested depth, and 0 is a real, meaningful value (an explicit
      // "restore to live" request) -- so this checks presence (`!= null`),
      // not truthiness, or the Live button's own `screenUrl(paneId, 0)`
      // call would silently drop the query string and hit the plain poll
      // URL instead.
      return historyDepth != null ? url + "?history=" + historyDepth : url;
    }

    // A pane's frame is a fixed grid: wrapping it destroys the box drawing, so
    // the only honest way to fit a wide frame on a narrow screen is to make
    // the type smaller, not to re-flow it. After each repaint the widest line
    // decides the size — screen width divided by that many columns. Below
    // FONT_MIN_PX the text stops being readable, so the box keeps its
    // horizontal scrollbar for that case rather than shrinking into a smear.
    // The floor is a readability floor, not a "how small can it go" one. Set
    // at 6px it was never reached on a desktop and always reached on a phone,
    // where a hundred-column frame cannot fit a phone's width at any size a
    // person can read — so every phone got the smallest type instead of the
    // wrapping that was meant to rescue it. 10px is about as small as a
    // monospace screen stays legible on a handset; below that the frame wraps.
    var FONT_MIN_PX = 10;
    var FONT_MAX_PX = 13;

    // Pane element -> the available width its last fit was computed against
    // (terminal-scroll-perf-1). A resize that leaves every pane's available
    // width unchanged — which happens constantly on a phone, where the URL
    // bar showing/hiding during page scroll fires `resize` on every frame —
    // has nothing to refit, so this cache lets the resize handler skip the
    // fit entirely instead of repeating it for no reason.
    var lastFitWidth = new WeakMap();

    // The box's own width is not a safe ceiling on its own: if anything
    // above it has already been pushed wider than the window, the box
    // inherits that width, the frame looks like it fits, and the sideways
    // scroll shows up on the page instead. The window is the one width
    // nothing can be wider than, so it caps the measurement. This alone is
    // one layout read (getComputedStyle + clientWidth) and is cheap enough
    // to also use as the resize handler's "did anything actually change"
    // check, separate from the more expensive scrollWidth measurement below.
    function availableWidth(el) {
      var style = window.getComputedStyle(el);
      var padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      var parent = el.parentElement;
      var ceiling = Math.min(
        el.clientWidth,
        parent ? parent.clientWidth : el.clientWidth,
        document.documentElement.clientWidth
      );
      return { available: ceiling - padding, padding: padding };
    }

    function fitScreenFont(el) {
      // Measure the frame's real width rather than counting its characters:
      // an emoji or a box-drawing glyph occupies two terminal cells while
      // counting as one character, and a glyph the mono font lacks is drawn
      // from a fallback of its own width — so any character-count estimate
      // runs short and the frame overflows anyway. `scrollWidth` is what the
      // browser actually laid out, and it is never wrong about it.
      el.classList.remove("term-screen--wrapped"); // measure unwrapped, always
      el.style.fontSize = FONT_MAX_PX + "px";
      var dims = availableWidth(el);
      var padding = dims.padding;
      var available = dims.available;
      if (available <= 0) return;
      lastFitWidth.set(el, available);

      var size = FONT_MAX_PX;
      // At most two forced-layout reads of scrollWidth per fit, not a
      // remeasure-every-pass loop: scrollWidth scales close to linearly with
      // font-size for a monospace grid, so one ratio lands within a
      // sub-pixel of the true fit. The common wide-screen case never even
      // reaches the second read.
      var needed = el.scrollWidth - padding; // read #1: width at FONT_MAX_PX
      if (needed > available) {
        size = Math.max(FONT_MIN_PX, FONT_MAX_PX * (available / needed));
        el.style.fontSize = size + "px";
        // Sub-pixel glyph advances can leave that ratio's guess a hair over
        // or under; this second read confirms it and, if it still
        // overflows, clamps the rest of the way to the floor arithmetically
        // (an estimate, not a third measurement — close enough for a
        // monospace grid) rather than looping to remeasure again.
        var confirmedNeeded = el.scrollWidth - padding; // read #2: confirm/clamp
        if (confirmedNeeded > available && size > FONT_MIN_PX) {
          var neededAtFloor = confirmedNeeded * (FONT_MIN_PX / size);
          size = FONT_MIN_PX;
          el.style.fontSize = size + "px";
          confirmedNeeded = neededAtFloor;
        }
        // At the floor the frame no longer fits at any readable size, so the
        // grid is already lost whatever we do. Wrapping is the cheapest way
        // to lose it: every character stays on screen and legible, at the
        // cost of the column alignment that narrow a screen could not have
        // shown anyway. Above the floor the grid is intact and stays
        // untouched.
        if (size <= FONT_MIN_PX && confirmedNeeded > available) {
          el.classList.add("term-screen--wrapped");
        }
      }
    }

    // One resize can change every pane's fit at once, but on a phone the
    // URL bar showing or hiding while the page scrolls fires `resize` on
    // nearly every frame — so a burst is coalesced into a single refit on
    // the next animation frame, and within that frame a pane whose
    // available width didn't actually move is skipped rather than refit for
    // nothing (terminal-scroll-perf-1).
    var resizeScheduled = false;
    window.addEventListener("resize", function () {
      if (resizeScheduled) return;
      resizeScheduled = true;
      requestAnimationFrame(function () {
        resizeScheduled = false;
        screens.forEach(function (el) {
          if (availableWidth(el).available === lastFitWidth.get(el)) return;
          fitScreenFont(el);
        });
      });
    });

    function pollOne(el) {
      var paneId = el.getAttribute("data-pane-id");
      if (viewingHistory[paneId]) return; // the operator is reading history; leave it alone
      var base = validTermBase(el.getAttribute("data-term-base"));
      // unassigned-poller-guard D1: neither a valid own base nor a page
      // projectId to fall back on — bail before the fetch ever builds
      // `/p/null/...` (the Unassigned page, wired by its own scoped IIFE
      // further down instead).
      if (!hasTarget(base, projectId)) return;
      if (inFlightScreen[paneId]) return; // a slow predecessor is still out; never stack a second fetch on it
      inFlightScreen[paneId] = true;
      fetch(screenUrl(paneId, null, base), { credentials: "same-origin" })
        .then(function (res) {
          // A 502 is the one status `herdr_down_response()` (server.rs) ever
          // sends, and only when herdr itself is unreachable — but the body
          // still has to say so, because a tunnel or proxy in front of this
          // page can hand back its own unrelated 502 HTML on a blip. Every
          // other failure (a thrown fetch below, any other status, a 502
          // whose body isn't that exact JSON) is treated as transient: the
          // pane keeps its last good screen and just gets marked stale,
          // never overwritten with wording that says the agent is gone.
          if (res.status === 502) {
            return res.json().then(function (body) {
              if (body && body.error === HERDR_DOWN_TEXT) {
                el.textContent = HERDR_DOWN_TEXT;
                el.classList.remove("term-screen--stale");
                // The next successful poll must always repaint, even if its
                // revision happens to match whatever was last drawn before
                // the outage — otherwise this banner never clears.
                delete lastRevision[paneId];
                return null;
              }
              el.classList.add("term-screen--stale");
              return null;
            });
          }
          if (!res.ok) {
            el.classList.add("term-screen--stale");
            return null;
          }
          return res.json();
        })
        .then(function (body) {
          if (!body) return;
          el.classList.remove("term-screen--stale");
          if (lastRevision[paneId] === body.revision) return; // unchanged, skip repaint
          lastRevision[paneId] = body.revision;
          // `body.text` is safe, pre-escaped ANSI-translated HTML — see the
          // doc comment above this IIFE.
          el.innerHTML = body.text;
          fitScreenFont(el);
        })
        .then(function () {
          // The request has settled (success or a handled non-ok status) —
          // clear here, not on headers, so the next tick can only ever
          // refetch once this poll is truly done.
          inFlightScreen[paneId] = false;
        })
        .catch(function () {
          // Thrown fetch (network blip, phone waking from sleep) or an
          // unparseable 502 body — none of these confirm herdr is actually
          // down, so the pane keeps whatever it last showed. Settle the flag
          // here too, or this pane's poller wedges forever after one error.
          inFlightScreen[paneId] = false;
          el.classList.add("term-screen--stale");
        });
    }

    function pollAll() {
      screens.forEach(pollOne);
    }

    pollAll();
    setInterval(pollAll, POLL_MS);

    // Scroll-history buttons (terminal-scroll-2, `herdr/pane_scroller.rs`;
    // made stateful by scroll-keep-position): "Load older" raises this
    // pane's own running depth by one and sends that ABSOLUTE depth — the
    // server now remembers where this pane last was
    // (`AppState::scroll_tracker`) and moves only the delta, so reaching
    // further back than the last press costs one hop server-side too, not
    // a full replay from live. This per-pane counter is still what supplies
    // that absolute depth on every call; the server no longer needs it to
    // restore between requests, only to know how far this call should go.
    // The response is applied unconditionally, never gated behind
    // `lastRevision`'s dedupe above: a history read's revision can coincide
    // with whatever this pane last polled live, and that coincidence must
    // never be read as "nothing changed, skip the repaint" the way a
    // genuine live-poll tick would.
    //
    // "Live" now sends its own explicit `?history=0` request rather than
    // just resetting local state: the server no longer restores a scrolled
    // pane on its own at the end of every history call, so without this
    // fetch the pane would stay escape-injection-scrolled server-side until
    // some later request happened to ask for depth 0 — visibly, the next
    // live poll would still show the stale escalated view for one more
    // tick. `viewingHistory[paneId]` stays true (the poller stays paused)
    // until this restore round trip actually lands, so depth 0 is reached
    // exactly once per "Live" press, never raced against the plain poller
    // also waking the pane up.
    Array.prototype.slice.call(document.querySelectorAll(".term-scroll[data-pane-id]")).forEach(function (group) {
      var paneId = group.getAttribute("data-pane-id");
      var card = group.closest(".term-pane");
      var screenEl = card ? card.querySelector(".term-screen[data-pane-id]") : null;
      if (!screenEl) return;
      // homepage-terminal-parity: the same per-element `data-term-base`
      // `pollOne` already reads (app.js:1033) — present only on the
      // homepage Terminals tab's `.term-screen` (`views.rs::screen_frame`),
      // absent (so `screenUrl` falls back to `projectId`) on the project
      // and Unassigned pages, unchanged.
      var base = validTermBase(screenEl.getAttribute("data-term-base"));
      // unassigned-poller-guard D1: same bail-out the reply form and key
      // group blocks already carry — no resolvable target for this pane's
      // history controls (the Unassigned page's own scoped IIFE further
      // down owns its panes instead), so Older/Newer/Live are never wired
      // to post `/p/null/...`.
      if (!hasTarget(base, projectId)) return;
      var olderBtn = group.querySelector('[data-scroll="older"]');
      var newerBtn = group.querySelector('[data-scroll="newer"]');
      var liveBtn = group.querySelector('[data-scroll="live"]');
      paneHistoryDepth[paneId] = 0; // 0 = live; how many PageUp-hops back this pane's last press reached

      // scroll-fab: Newer's disabled state is a plain function of the depth
      // — disabled exactly at depth 0 (already live, nothing to request),
      // enabled the moment a press takes it above 0. Called at init and
      // again everywhere the depth changes (Older's, Newer's and Live's own
      // handlers below) rather than left implicit in each handler's own
      // logic, so this one line is the single place that rule lives.
      function updateNewerDisabled() {
        if (newerBtn) newerBtn.disabled = paneHistoryDepth[paneId] === 0;
      }
      updateNewerDisabled();

      if (olderBtn) {
        olderBtn.addEventListener("click", function () {
          viewingHistory[paneId] = true; // pause the poller before the round trip, not after
          var requestedDepth = paneHistoryDepth[paneId] + 1;
          fetch(screenUrl(paneId, requestedDepth, base), { credentials: "same-origin" })
            .then(function (res) {
              return res.ok ? res.json() : null;
            })
            .then(function (body) {
              if (!body) return;
              paneHistoryDepth[paneId] = requestedDepth;
              lastRevision[paneId] = body.revision;
              screenEl.innerHTML = body.text;
              fitScreenFont(screenEl);
              updateNewerDisabled();
            })
            .catch(function () {});
        });
      }

      if (newerBtn) {
        newerBtn.addEventListener("click", function () {
          // scroll-fab: Newer walks one page back toward live through the
          // same request path Older uses (`screenUrl`, `paneHistoryDepth`,
          // `viewingHistory`) — never below depth 0, which is live itself
          // and is already what Newer's own disabled state guards against
          // requesting.
          viewingHistory[paneId] = true; // pause the poller before the round trip, not after
          var requestedDepth = Math.max(paneHistoryDepth[paneId] - 1, 0);
          fetch(screenUrl(paneId, requestedDepth, base), { credentials: "same-origin" })
            .then(function (res) {
              return res.ok ? res.json() : null;
            })
            .then(function (body) {
              if (!body) return;
              paneHistoryDepth[paneId] = requestedDepth;
              lastRevision[paneId] = body.revision;
              screenEl.innerHTML = body.text;
              fitScreenFont(screenEl);
              updateNewerDisabled();
              if (requestedDepth === 0) {
                // Reaching depth 0 through Newer is the same live end-state
                // the Live button's own handler reaches below — resume the
                // poller exactly as its success path does.
                viewingHistory[paneId] = false;
                screenEl.scrollTop = screenEl.scrollHeight;
              }
            })
            .catch(function () {});
        });
      }

      if (liveBtn) {
        liveBtn.addEventListener("click", function () {
          // Keep the poller paused (viewingHistory stays true) until this
          // explicit depth-0 request actually lands -- reaching depth 0 is
          // this press's job alone, never left to race the next poll tick.
          paneHistoryDepth[paneId] = 0;
          updateNewerDisabled(); // Live always lands at depth 0, so Newer disables immediately
          fetch(screenUrl(paneId, 0, base), { credentials: "same-origin" })
            .then(function (res) {
              return res.ok ? res.json() : null;
            })
            .then(function (body) {
              if (body) {
                lastRevision[paneId] = body.revision;
                screenEl.innerHTML = body.text;
                fitScreenFont(screenEl);
              }
              viewingHistory[paneId] = false;
              screenEl.scrollTop = screenEl.scrollHeight;
            })
            .catch(function () {
              // Never strand the pane paused if the restore request itself
              // failed -- the regular poller resuming is the fallback.
              viewingHistory[paneId] = false;
              screenEl.scrollTop = screenEl.scrollHeight;
            });
        });
      }
    });

    // scroll-keep-position review fix (C): a best-effort last-gasp restore
    // when the page is about to go away (tab closed, backgrounded on
    // mobile, navigated away). `pagehide` fires reliably on both desktop
    // and mobile browsers (unlike `beforeunload`, which mobile Safari/
    // Chrome often skip entirely); `visibilitychange` going `"hidden"`
    // additionally catches the "backgrounded, never actually unloaded"
    // state a phone can leave a tab in indefinitely. `keepalive: true`
    // lets the request survive the page's own teardown.
    //
    // BEST-EFFORT ONLY: no response is read, no retry, no confirmation the
    // server ever received it -- the server-side idle-TTL sweep (review fix
    // C, `server.rs`'s `scroll_aware_read`, run on every `/screen` request
    // for ANY pane) is the real, load-bearing guarantee that an abandoned
    // pane never stays parked forever; this is only a faster path for the
    // common case where the browser gets to run it at all.
    function restoreEveryScrolledPaneBestEffort() {
      Object.keys(paneHistoryDepth).forEach(function (paneId) {
        if (paneHistoryDepth[paneId] > 0) {
          fetch(screenUrl(paneId, 0), { credentials: "same-origin", keepalive: true }).catch(function () {});
        }
      });
    }
    window.addEventListener("pagehide", restoreEveryScrolledPaneBestEffort);
    document.addEventListener("visibilitychange", function () {
      if (document.visibilityState === "hidden") {
        restoreEveryScrolledPaneBestEffort();
      }
    });
  })();

  // Transcript poll (agent-terminal-16, D9): each pane's `.term-transcript`
  // viewport, on the separate Transcript tab, polls its own
  // `/p/:id/_terminal/:pane_id/transcript` endpoint on the same fixed
  // interval as the screen poller above. The cursor the endpoint returns is
  // held here, client-side, per pane — nothing about the transcript is ever
  // stored server-side (`waggledance-core`'s transcript module doc) — and every
  // poll *appends* the newly returned records rather than replacing the
  // viewport's contents, so nothing already shown is ever lost between
  // polls, unlike the screen poller's full-repaint `innerHTML` above.
  //
  // `body.lines` already carries safe, pre-escaped HTML from waggledance-core's
  // ansi translator — the same one the screen poller uses — so each line is
  // assigned via `innerHTML`, never `textContent`, matching that precedent.
  //
  // D6: `body.available === false` means this pane's agent has written no
  // transcript yet — a named state is shown once and left alone, never
  // repainted back to "Loading…" and never left blank as if broken.
  //
  // Two defects fixed here (independent review, agent-terminal-20), plus a
  // fix to the fix (independent review, agent-terminal-22):
  // (1) `pollOne` used to fire on every `POLL_MS` tick with no guard against
  // an outstanding request — a poll slower than `POLL_MS` (a slow herdr, a
  // large tail) let the next tick fire with the *same* `cursors[paneId]`,
  // so both responses carried the same records and both got appended,
  // showing every record twice. `inFlight` skips a tick whose predecessor
  // hasn't resolved yet, the same cursor is then only ever read once.
  // agent-terminal-20's first version cleared `inFlight` in the *headers*
  // handler — before the cursor below had advanced — so a tick landing in
  // that window still refetched with the stale cursor and still
  // double-appended, the exact defect the flag exists to prevent. The flag
  // now clears only once the request has fully settled: in a `.then`
  // chained *after* the cursor advance on success, and in `.catch` on
  // outright failure — never on headers alone, and independently on both
  // paths, so one path left uncleared can never wedge the other.
  // (2) every non-ok response used to be swallowed as "nothing to do",
  // leaving the last-good content on screen forever while silently
  // re-sending the same cursor — indistinguishable from an idle agent. A
  // named state now always replaces the viewport's own message area,
  // distinguishing "this pane is gone" (the transcript route answers a
  // reasoned JSON 404 when the terminal is switched off, the project is
  // gone, or the pane no longer exists — no session guard is left to fail)
  // from a transient failure (a non-404 error status, or the request
  // failing outright), so the operator knows to check the pane and the
  // Settings switch rather than assume the transcript itself is stuck.
  // Recovering (any next successful, parsed response) clears the named
  // state without disturbing lines already appended.
  (function () {
    var main = document.querySelector("main.fg-page[data-project-id]");
    if (!main) return;
    var projectId = main.getAttribute("data-project-id");
    var viewports = Array.prototype.slice.call(document.querySelectorAll(".term-transcript[data-pane-id]"));
    if (!projectId || !viewports.length) return;

    var POLL_MS = 1500;
    var NO_TRANSCRIPT_TEXT = "No transcript yet for this pane.";
    var SESSION_EXPIRED_TEXT = "This pane is no longer reachable — it may have been removed, or the terminal switched off in Settings.";
    var TRANSCRIPT_ERROR_TEXT = "Couldn't reach the transcript — retrying…";
    var cursors = {}; // pane id -> cursor to resume from on the next poll
    var started = {}; // pane id -> the viewport's placeholder has been cleared
    var inFlight = {}; // pane id -> a poll for this pane is still outstanding
    var errorEl = {}; // pane id -> the named-state element currently shown, if any

    function transcriptUrl(paneId, cursor) {
      var url = "/p/" + encodeURIComponent(projectId) + "/_terminal/" + encodeURIComponent(paneId) + "/transcript";
      return cursor ? url + "?cursor=" + encodeURIComponent(cursor) : url;
    }

    function appendLines(el, lines) {
      lines.forEach(function (html) {
        var line = document.createElement("div");
        line.className = "term-transcript__line";
        // `html` is safe, pre-escaped markup — see the doc comment above
        // this IIFE.
        line.innerHTML = html;
        el.appendChild(line);
      });
      if (lines.length) el.scrollTop = el.scrollHeight;
    }

    // Shows `text` as a standing, visible state for this pane — replacing
    // whatever named state (if any) is already shown, never appended beside
    // accumulated transcript lines and never silently dropped.
    function showState(el, paneId, text) {
      var node = errorEl[paneId];
      if (!node) {
        node = document.createElement("div");
        node.className = "term-transcript__line term-transcript__state";
        el.appendChild(node);
        errorEl[paneId] = node;
      }
      node.textContent = text;
      el.scrollTop = el.scrollHeight;
    }

    function clearState(paneId) {
      var node = errorEl[paneId];
      if (node && node.parentNode) node.parentNode.removeChild(node);
      errorEl[paneId] = null;
    }

    function pollOne(el) {
      var paneId = el.getAttribute("data-pane-id");
      if (inFlight[paneId]) return; // a slow predecessor is still out; never race it with the same cursor
      inFlight[paneId] = true;
      fetch(transcriptUrl(paneId, cursors[paneId]), { credentials: "same-origin" })
        .then(function (res) {
          // Headers have arrived, but the request has not settled — the
          // in-flight flag stays set until the body below has been read and
          // the cursor (if any) has advanced. Clearing here would let a poll
          // tick land before the body finishes parsing, refetch with the
          // same cursor, and append the same records twice.
          if (res.status === 404) {
            // The transcript route has no session to fail — a 404 here means
            // the terminal switch is off, the project is gone, or this pane
            // no longer exists.
            showState(el, paneId, SESSION_EXPIRED_TEXT);
            return null;
          }
          if (!res.ok) {
            showState(el, paneId, TRANSCRIPT_ERROR_TEXT);
            return null;
          }
          return res.json();
        })
        .then(function (body) {
          if (!body) return;
          clearState(paneId);
          if (body.available === false) {
            if (!started[paneId]) el.textContent = NO_TRANSCRIPT_TEXT;
            return;
          }
          if (!started[paneId]) {
            el.textContent = "";
            started[paneId] = true;
          }
          cursors[paneId] = body.cursor;
          appendLines(el, body.lines || []);
        })
        .then(function () {
          // The request has settled — success or a handled non-ok status —
          // and any cursor advance above has already happened. Clear here,
          // not on headers, so the next tick can only ever refetch once this
          // poll is truly done.
          inFlight[paneId] = false;
        })
        .catch(function () {
          // The request failed outright (network error, a rejected
          // `res.json()`). Settle the flag here too, or this pane's poller
          // wedges forever after one error — no other path clears it.
          inFlight[paneId] = false;
          showState(el, paneId, TRANSCRIPT_ERROR_TEXT);
        });
    }

    function pollAll() {
      viewports.forEach(pollOne);
    }

    pollAll();
    setInterval(pollAll, POLL_MS);
  })();

  // Terminal reply + keys (agent-terminal-9, D3): posts free text and named
  // keys back to a pane. Send≠submit stays two distinct actions here too —
  // "Send" posts with `submit: true` (herdr presses Enter as its own,
  // separate call), "Stage" posts with `submit: false` so the text lands in
  // the pane's composer without being sent. After either send, this never
  // repaints the screen itself — the existing `.term-screen` poller above
  // already runs on its own interval and will pick up the change on its
  // next tick, per this cell's instruction not to invent a second refresh
  // mechanism.
  //
  // terminal-image-attach (D1/D2): the same form also owns the attach
  // control, when the page rendered one (`views.rs::pane_controls` gates the
  // markup to project pages only, per plan finding 7 — the Unassigned page
  // has no `.term-attach` box for this loop to find, so all of the wiring
  // below is a no-op there). Picker, drag-drop on the form, and paste in the
  // textarea all feed one `upload` function; each 200 adds a removable chip
  // holding the returned path, any refusal shows the server's own message
  // and adds no chip. "Send" (the submit event and the Ctrl+Enter
  // keybinding — the two `submit: true` paths) composes ONE message out of
  // the prompt text and every remaining chip's path and clears the chips
  // once that send lands; "Stage" keeps sending the textarea alone, and
  // neither keybinding changes shape.
  //
  // homepage-terminals: same `data-term-base`-wins-outright branch the
  // screen poller above already carries (`views.rs::pane_controls`'s doc) —
  // each `.term-reply`/`.term-keys`/`.term-attach` element's own base, read
  // once per element below and threaded through every call site instead of
  // one closure-captured `projectId`, since the home page's Terminals tab
  // has panes from more than one project (or none, D3) on the same page.
  // `main.fg-page[data-project-id]` is still read for the project and
  // Unassigned pages' own fallback path; the home page renders no such
  // attribute, so `projectId` is `null` there and every element on it must
  // carry its own `data-term-base` or post nowhere.
  (function () {
    var main = document.querySelector("main.fg-page[data-project-id]");
    var projectId = main ? main.getAttribute("data-project-id") : null;
    var forms = document.querySelectorAll(".term-reply[data-pane-id]");
    var keyGroups = document.querySelectorAll(".term-keys[data-pane-id]");
    if (!forms.length && !keyGroups.length) return;

    function inputUrl(paneId, base) {
      return base
        ? base + "/input"
        : "/p/" + encodeURIComponent(projectId) + "/_terminal/" + encodeURIComponent(paneId) + "/input";
    }

    function keysUrl(paneId, base) {
      return base
        ? base + "/keys"
        : "/p/" + encodeURIComponent(projectId) + "/_terminal/" + encodeURIComponent(paneId) + "/keys";
    }

    function attachUrl(paneId, base) {
      return base
        ? base + "/attach"
        : "/p/" + encodeURIComponent(projectId) + "/_terminal/" + encodeURIComponent(paneId) + "/attach";
    }

    function postJson(url, body) {
      return fetch(url, {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
    }

    function sendReply(paneId, text, submit, input, base) {
      if (!text) return;
      postJson(inputUrl(paneId, base), { text: text, submit: submit })
        .then(function (res) {
          if (res.ok && input) input.value = "";
        })
        .catch(function () {});
    }

    // paneId -> [{ path: "<absolute path>" }, ...], the chips currently
    // staged for that pane's next send.
    var chips = {};

    // paneId -> running "Image N" counter, reset with clearChips so the
    // next message's chips start over at Image 1. A failed upload still
    // consumes its number — labels within one message stay unique even
    // when a chip is removed or an upload dies mid-flight.
    var chipSeq = {};

    function chipsFor(paneId) {
      if (!chips[paneId]) chips[paneId] = [];
      return chips[paneId];
    }

    function nextChipLabel(paneId) {
      chipSeq[paneId] = (chipSeq[paneId] || 0) + 1;
      return "Image " + chipSeq[paneId];
    }

    function clearChips(paneId, chipList) {
      chips[paneId] = [];
      chipSeq[paneId] = 0;
      if (chipList) chipList.innerHTML = "";
    }

    function showAttachError(errorEl, message) {
      if (!errorEl) return;
      errorEl.textContent = message;
      errorEl.hidden = false;
    }

    function clearAttachError(errorEl) {
      if (!errorEl) return;
      errorEl.hidden = true;
      errorEl.textContent = "";
    }

    // One chip per upload, appended the moment the upload starts so the
    // user sees progress immediately: a dimmed "Image N (uploading…)"
    // pending state that `resolve` turns into the staged, removable chip
    // once the server answers with the stored path, and `reject` removes
    // again on failure. The visible label stays the short "Image N" —
    // the full path rides the label's `title` and the staged list, so
    // `composeMessage` still sends real paths, and a removed chip's path
    // never rides a later send because `composeMessage` only ever reads
    // `chipsFor(paneId)`.
    function addPendingChip(paneId, chipList) {
      var name = nextChipLabel(paneId);
      var li = document.createElement("li");
      li.className = "term-attach__chip term-attach__chip--pending";
      var label = document.createElement("span");
      label.className = "term-attach__chip-label";
      label.textContent = name + " (uploading…)";
      li.appendChild(label);
      chipList.appendChild(li);
      return {
        resolve: function (path) {
          chipsFor(paneId).push({ path: path });
          li.className = "term-attach__chip";
          label.textContent = name;
          label.title = path;
          var remove = document.createElement("button");
          remove.type = "button";
          remove.className = "term-attach__chip-remove";
          remove.setAttribute("aria-label", "Remove " + name);
          remove.textContent = "×";
          remove.addEventListener("click", function () {
            var list = chipsFor(paneId);
            var at = list.findIndex(function (c) {
              return c.path === path;
            });
            if (at !== -1) list.splice(at, 1);
            li.remove();
          });
          li.appendChild(remove);
        },
        reject: function () {
          li.remove();
        },
      };
    }

    // One raw-body upload per file (D1) — the endpoint this cell wires
    // against takes one file per request, `Content-Type` carrying the
    // file's own MIME type.
    function upload(paneId, file, chipList, errorEl, base) {
      clearAttachError(errorEl);
      var chip = addPendingChip(paneId, chipList);
      return fetch(attachUrl(paneId, base), {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": file.type || "application/octet-stream" },
        body: file,
      })
        .then(function (res) {
          return res
            .json()
            .catch(function () {
              return null;
            })
            .then(function (body) {
              if (!res.ok) {
                chip.reject();
                showAttachError(errorEl, (body && body.error) || "upload failed");
                return;
              }
              if (body && body.path) chip.resolve(body.path);
              else chip.reject();
            });
        })
        .catch(function () {
          chip.reject();
          showAttachError(errorEl, "upload failed");
        });
    }

    // terminal-image-attach-3 P3: mirrors the server's own `ATTACH_MAX_BYTES`
    // (`server.rs`) so an oversized drop is refused client-side, with the
    // same visible error surface a server refusal uses, before any bytes
    // ever leave the browser.
    var ATTACH_MAX_BYTES = 10 * 1024 * 1024;

    function uploadFiles(paneId, files, chipList, errorEl, base) {
      Array.prototype.slice.call(files || []).forEach(function (file) {
        if (file.size > ATTACH_MAX_BYTES) {
          showAttachError(errorEl, "upload exceeds the 10 MB limit");
          return;
        }
        upload(paneId, file, chipList, errorEl, base);
      });
    }

    // terminal-image-attach-3 P3: a path containing whitespace (a `$HOME` or
    // `$XDG_RUNTIME_DIR` with a space in it) would otherwise fall apart when
    // space-joined with its neighbors; quoting only the paths that need it
    // keeps the common case bare.
    function quotePathIfNeeded(path) {
      return /\s/.test(path) ? '"' + path + '"' : path;
    }

    // The composed message a "Send" carries: prompt text, a newline when
    // both parts exist, then every remaining chip's path space-joined (D2),
    // double-quoting any path that itself contains whitespace.
    function composeMessage(paneId, promptText) {
      var paths = chipsFor(paneId)
        .map(function (c) {
          return quotePathIfNeeded(c.path);
        })
        .join(" ");
      if (promptText && paths) return promptText + "\n" + paths;
      return promptText || paths;
    }

    function sendComposed(paneId, promptText, input, chipList, base) {
      var text = composeMessage(paneId, promptText);
      if (!text) return;
      postJson(inputUrl(paneId, base), { text: text, submit: true })
        .then(function (res) {
          if (res.ok) {
            if (input) input.value = "";
            clearChips(paneId, chipList);
          }
        })
        .catch(function () {});
    }

    Array.prototype.slice.call(forms).forEach(function (form) {
      var paneId = form.getAttribute("data-pane-id");
      var base = validTermBase(form.getAttribute("data-term-base"));
      // unassigned-poller-guard D1: no resolvable target for this form —
      // skip wiring it entirely (covers input AND, when rendered, attach:
      // both post through this same base). The Unassigned page's own
      // scoped IIFE further down already owns this form; this only stops
      // the second, unscoped copy from double-posting into `/p/null/...`
      // alongside it.
      if (!hasTarget(base, projectId)) return;
      var input = form.querySelector(".term-reply__text");
      var stageBtn = form.querySelector(".term-reply__stage");
      var approveBtn = form.querySelector(".term-reply__approve");
      var attachBox = form.querySelector(".term-attach[data-pane-id]");
      var fileInput = attachBox && attachBox.querySelector(".term-attach__input");
      var attachBtn = attachBox && attachBox.querySelector(".term-attach__btn");
      var chipList = attachBox && attachBox.querySelector(".term-attach__chips");
      var errorEl = attachBox && attachBox.querySelector(".term-attach__error");

      form.addEventListener("submit", function (ev) {
        ev.preventDefault();
        sendComposed(paneId, input.value, input, chipList, base);
      });

      // The reply box is a textarea, so Enter belongs to the text — it opens
      // a new line the way it does anywhere else. Ctrl+Enter (Cmd+Enter on a
      // Mac) is what a single-line field's bare Enter used to be: send.
      if (input) {
        input.addEventListener("keydown", function (ev) {
          if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
            ev.preventDefault();
            sendComposed(paneId, input.value, input, chipList, base);
          }
        });

        // Clipboard paste (D1): any pasted item that is a file (an image
        // copied from elsewhere) feeds the same upload path as the picker
        // and drag-drop, instead of landing as text/garbage in the textarea.
        input.addEventListener("paste", function (ev) {
          var items = ev.clipboardData && ev.clipboardData.items;
          if (!items || !attachBox) return;
          var files = [];
          Array.prototype.slice.call(items).forEach(function (item) {
            if (item.kind !== "file") return;
            var file = item.getAsFile();
            if (file) files.push(file);
          });
          if (files.length) {
            ev.preventDefault();
            uploadFiles(paneId, files, chipList, errorEl, base);
          }
        });
      }

      if (stageBtn) {
        stageBtn.addEventListener("click", function () {
          sendReply(paneId, input.value, false, input, base);
        });
      }

      if (approveBtn) {
        approveBtn.addEventListener("click", function () {
          postJson(inputUrl(paneId, base), { text: "Approve", submit: true }).catch(function () {});
        });
      }

      if (attachBox) {
        if (attachBtn && fileInput) {
          attachBtn.addEventListener("click", function () {
            fileInput.click();
          });
          fileInput.addEventListener("change", function () {
            uploadFiles(paneId, fileInput.files, chipList, errorEl, base);
            fileInput.value = "";
          });
        }

        // Drag-drop onto the whole composer (D1), not just the file input.
        form.addEventListener("dragover", function (ev) {
          if (ev.dataTransfer && Array.prototype.indexOf.call(ev.dataTransfer.types || [], "Files") !== -1) {
            ev.preventDefault();
          }
        });
        form.addEventListener("drop", function (ev) {
          var files = ev.dataTransfer && ev.dataTransfer.files;
          if (files && files.length) {
            ev.preventDefault();
            uploadFiles(paneId, files, chipList, errorEl, base);
          }
        });
      }
    });

    Array.prototype.slice.call(keyGroups).forEach(function (group) {
      var paneId = group.getAttribute("data-pane-id");
      var base = validTermBase(group.getAttribute("data-term-base"));
      // unassigned-poller-guard D1: same bail-out as the reply form above —
      // no target, no key posts wired for this group.
      if (!hasTarget(base, projectId)) return;
      Array.prototype.slice.call(group.querySelectorAll("button[data-key]")).forEach(function (btn) {
        btn.addEventListener("click", function () {
          var key = btn.getAttribute("data-key");
          if (!key) return;
          postJson(keysUrl(paneId, base), { keys: [key] }).catch(function () {});
        });
      });
    });
  })();

  // Unassigned-page terminal poll/reply/keys (D4/D5/D6; folded into this
  // file from views.rs's own `UNASSIGNED_TERMINAL_SCRIPT` const by
  // backlog-groom-2-4). Scoped to `.unassigned-panes` so it never touches a
  // project page's own panes, same as before the fold. The shared screen
  // poller and reply/keys wiring above deliberately skip this page
  // (`hasTarget` above finds neither a `data-project-id` nor a
  // `data-term-base` here, `unassigned-poller-guard` D1) — this pane group
  // belongs to no project id, so every route below is built from this
  // page's own `data-unassigned-base` on `<main>`
  // (`views.rs::unassigned_terminal_page`) rather than the shared
  // `/p/:id/...` shape. Bails immediately on every other page, where that
  // attribute is absent.
  (function () {
    var main = document.querySelector("main[data-unassigned-base]");
    if (!main) return;
    var BASE = main.getAttribute("data-unassigned-base");
    var POLL_MS = 1500;
    var HERDR_DOWN_TEXT = "herdr is not running";
    var lastRevision = {};

    function screenUrl(paneId) {
      return BASE + "/" + encodeURIComponent(paneId) + "/screen";
    }
    function inputUrl(paneId) {
      return BASE + "/" + encodeURIComponent(paneId) + "/input";
    }
    function keysUrl(paneId) {
      return BASE + "/" + encodeURIComponent(paneId) + "/keys";
    }

    function pollOne(el) {
      var paneId = el.getAttribute("data-pane-id");
      fetch(screenUrl(paneId), { credentials: "same-origin" })
        .then(function (res) {
          // A 502 is the one status `herdr_down_response()` (server.rs) ever
          // sends, and only when herdr itself is unreachable — but the body
          // still has to say so, because a tunnel or proxy in front of this
          // page can hand back its own unrelated 502 HTML on a blip. Every
          // other failure (a thrown fetch below, any other status, a 502
          // whose body isn't that exact JSON) is treated as transient: the
          // pane keeps its last good screen and just gets marked stale, never
          // overwritten with wording that says the agent is gone.
          if (res.status === 502) {
            return res.json().then(function (body) {
              if (body && body.error === HERDR_DOWN_TEXT) {
                el.textContent = HERDR_DOWN_TEXT;
                el.classList.remove("term-screen--stale");
                // The next successful poll must always repaint, even if its
                // revision happens to match whatever was last drawn before
                // the outage — otherwise this banner never clears.
                delete lastRevision[paneId];
                return null;
              }
              el.classList.add("term-screen--stale");
              return null;
            });
          }
          if (!res.ok) { el.classList.add("term-screen--stale"); return null; }
          return res.json();
        })
        .then(function (body) {
          if (!body) return;
          el.classList.remove("term-screen--stale");
          if (lastRevision[paneId] === body.revision) return;
          lastRevision[paneId] = body.revision;
          // `body.text` is safe, pre-escaped HTML from waggledance-core's ansi
          // translator (agent-terminal-12) — never the raw pane text — so
          // `innerHTML` here renders ANSI colour/attribute markup rather than
          // showing literal escape characters.
          el.innerHTML = body.text;
        })
        .catch(function () {
          // Thrown fetch (network blip, phone waking from sleep) or an
          // unparseable 502 body — none of these confirm herdr is actually
          // down, so the pane keeps whatever it last showed.
          el.classList.add("term-screen--stale");
        });
    }

    function pollAll() {
      Array.prototype.slice
        .call(document.querySelectorAll(".unassigned-panes .term-screen[data-pane-id]"))
        .forEach(pollOne);
    }
    pollAll();
    setInterval(pollAll, POLL_MS);

    function postJson(url, body) {
      return fetch(url, {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
    }

    function sendReply(paneId, text, submit, input) {
      if (!text) return;
      postJson(inputUrl(paneId), { text: text, submit: submit })
        .then(function (res) { if (res.ok && input) input.value = ""; })
        .catch(function () {});
    }

    Array.prototype.slice
      .call(document.querySelectorAll(".unassigned-panes .term-reply[data-pane-id]"))
      .forEach(function (form) {
        var paneId = form.getAttribute("data-pane-id");
        var input = form.querySelector(".term-reply__text");
        var stageBtn = form.querySelector(".term-reply__stage");
        var approveBtn = form.querySelector(".term-reply__approve");
        form.addEventListener("submit", function (ev) {
          ev.preventDefault();
          sendReply(paneId, input.value, true, input);
        });
        if (input) {
          input.addEventListener("keydown", function (ev) {
            if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
              ev.preventDefault();
              sendReply(paneId, input.value, true, input);
            }
          });
        }
        if (stageBtn) {
          stageBtn.addEventListener("click", function () {
            sendReply(paneId, input.value, false, input);
          });
        }
        if (approveBtn) {
          approveBtn.addEventListener("click", function () {
            postJson(inputUrl(paneId), { text: "Approve", submit: true }).catch(function () {});
          });
        }
      });

    Array.prototype.slice
      .call(document.querySelectorAll(".unassigned-panes .term-keys[data-pane-id]"))
      .forEach(function (group) {
        var paneId = group.getAttribute("data-pane-id");
        Array.prototype.slice.call(group.querySelectorAll("button[data-key]")).forEach(function (btn) {
          btn.addEventListener("click", function () {
            var key = btn.getAttribute("data-key");
            if (!key) return;
            postJson(keysUrl(paneId), { keys: [key] }).catch(function () {});
          });
        });
      });
  })();

  // Terminal creation controls (agent-terminal-13; folded into this file
  // from views.rs's own `TERMINAL_CREATE_SCRIPT` const by
  // backlog-groom-2-4): POSTs "New shell"/preset clicks to
  // `create/pane`/`create/agent` and reloads the page on success so the
  // freshly created pane joins the screen poller above on the next render.
  // Scoped to `.term-create[data-project-id]`, which `views.rs`
  // (`terminal_create_controls`) renders at most once per page — the
  // project terminal page's own control, or the homepage Terminals tab's,
  // keyed to the selected pane's project — never both on the same page, so
  // this never double-binds a listener.
  (function () {
    var boxes = document.querySelectorAll(".term-create[data-project-id]");
    if (!boxes.length) return;

    function postJson(url, body) {
      return fetch(url, {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
    }
    function afterCreate(promise, failMsg) {
      promise
        .then(function (res) {
          if (res.ok) {
            location.reload();
            return;
          }
          return res.json().then(function (b) {
            alert((b && b.error) || failMsg);
          });
        })
        .catch(function () {
          alert(failMsg);
        });
    }
    Array.prototype.slice
      .call(boxes)
      .forEach(function (box) {
        var pid = box.getAttribute("data-project-id");
        var paneBtn = box.querySelector(".term-create__pane");
        if (paneBtn) {
          paneBtn.addEventListener("click", function () {
            afterCreate(
              postJson("/p/" + encodeURIComponent(pid) + "/_terminal/create/pane", {}),
              "could not start a shell"
            );
          });
        }
        Array.prototype.slice
          .call(box.querySelectorAll(".term-create__agent[data-preset]"))
          .forEach(function (btn) {
            btn.addEventListener("click", function () {
              afterCreate(
                postJson("/p/" + encodeURIComponent(pid) + "/_terminal/create/agent", {
                  preset: btn.getAttribute("data-preset"),
                }),
                "could not start an agent"
              );
            });
          });
      });
  })();

  // Collapsible menus (the top bar's navigation, a terminal page's pane
  // bar): the checkbox already owns open/closed, so this only adds the two
  // things the markup has no opinion about — Escape, and a press outside the
  // panel. Without this script every menu still opens and closes from its
  // own label.
  (function () {
    var menus = Array.prototype.slice.call(document.querySelectorAll(".js-menu"));
    if (!menus.length) return;
    function toggleOf(menu) {
      return menu.querySelector("input[type=checkbox]");
    }
    function closeAll(except) {
      menus.forEach(function (menu) {
        if (menu === except) return;
        var t = toggleOf(menu);
        if (t && t.checked) t.checked = false;
      });
    }
    document.addEventListener("click", function (e) {
      var inside = null;
      menus.forEach(function (menu) {
        if (menu.contains(e.target)) inside = menu;
      });
      // agents-drawer-global: a label can target a checkbox that lives in a
      // DIFFERENT .js-menu — the topbar menu's own "Agents" entry
      // (views.rs::topbar_full) is a <label for="agent-drawer-toggle">
      // sitting inside the topbar menu, but that id belongs to the drawer's
      // own .agent-drawer menu (views.rs::agent_switch_drawer). Clicking it
      // checks the drawer's checkbox — opening the drawer — while `inside`
      // above still reads the topbar menu, since that's where the click
      // physically landed; closeAll(inside) would then immediately re-close
      // the drawer it just opened. Resolve label -> checkbox -> that
      // checkbox's OWN menu, and let it override `inside` when tracked, so
      // the menu whose checkbox the click actually flipped is the one that
      // stays open. Every other menu's label targets its own checkbox (same
      // menu), so this is a no-op there.
      var label = e.target.closest && e.target.closest("label[for]");
      if (label) {
        var targetCheckbox = document.getElementById(label.getAttribute("for"));
        var targetMenu = targetCheckbox && targetCheckbox.closest(".js-menu");
        if (targetMenu && menus.indexOf(targetMenu) !== -1) inside = targetMenu;
      }
      closeAll(inside);
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape") closeAll(null);
    });
  })();

  // agent-switch-drawer-2: the cross-project agent feed
  // (`views.rs::agent_switch_drawer` renders the panel and its toggle;
  // `GET /api/agents` is the feed itself). Polled only while the drawer is
  // open — same fetch/repaint idiom as the terminal screen poller above,
  // but gated on the checkbox's own state, since this list is a jump menu
  // rather than a live view anything else on the page depends on.
  (function () {
    var toggle = document.getElementById("agent-drawer-toggle");
    var list = document.querySelector("[data-agent-drawer-list]");
    if (!toggle || !list) return;

    // home-terminal-parity-2: the one thing that tells the homepage
    // Terminals tab's own drawer instance apart from the project terminal
    // page's (`views.rs::agent_switch_drawer`'s `homepage` flag) — read
    // once, since the attribute never changes for the lifetime of this
    // static markup.
    // home-terminal-header-2: since both instances now render the same
    // project-grouped shape, this flag decides exactly one thing — where a
    // row leads (`agentRow`).
    var homepage = list.hasAttribute("data-agent-drawer-homepage");

    var POLL_MS = 5000;
    var timer = null;

    // The same mapping `views.rs::status_pill` applies server-side: `done`
    // reads ready, `working` reads warn, `blocked` reads blocked, and every
    // other status (`idle`, `unknown`, or anything this list has never seen
    // before) keeps the bare, unmodified dot rather than borrowing another
    // state's colour.
    function pillModifier(status) {
      if (status === "done") return " fg-status--ready";
      if (status === "working") return " fg-status--warn";
      if (status === "blocked") return " fg-status--blocked";
      return "";
    }

    // home-terminal-parity-2: blocked before working before the rest — the
    // same D4 rank `views.rs::terminals_status_rank` applies server-side to
    // this tab's own pane inventory — used to order rows *within* each
    // project group. home-terminal-header-2: that is now every group, on
    // both pages, since status sections are gone.
    function statusRank(status) {
      if (status === "blocked") return 0;
      if (status === "working") return 1;
      return 2;
    }

    function agentRow(agent) {
      var item = document.createElement("a");
      item.className = "fg-menu__item agent-drawer__item";
      // home-terminal-parity-2: the homepage instance links to its own tab
      // rather than the agent's own project page — `agent.url` still wins
      // outright on the project page (`homepage: false`, unchanged).
      item.href = homepage ? "/?tab=terminals&pane=" + encodeURIComponent(agent.pane_id) : agent.url;

      // Two-line row: status + pane address on the head line, then the
      // agent's terminal title (what it is doing) on its own line below.
      var head = document.createElement("span");
      head.className = "agent-drawer__head";

      var pill = document.createElement("span");
      pill.className = "fg-status" + pillModifier(agent.status);
      var dot = document.createElement("span");
      dot.className = "fg-status__dot";
      pill.appendChild(dot);
      pill.appendChild(document.createTextNode(agent.status));
      head.appendChild(pill);

      var name = document.createElement("span");
      name.textContent = agent.name;
      head.appendChild(name);

      // Rows already sit under their project's section heading
      // (`renderByProject`), so this carries only the pane address.
      var suffix = document.createElement("span");
      suffix.className = "agent-drawer__suffix";
      suffix.textContent = agent.workspace + ":" + agent.tab;
      head.appendChild(suffix);

      item.appendChild(head);

      // drawer-title-1: the agent's own terminal title — what it is doing
      // right now (`AgentPaneRow.title`). Skipped when empty or when it
      // merely repeats the name, so a titleless agent's row stays one line.
      if (agent.title && agent.title !== agent.name) {
        var title = document.createElement("span");
        title.className = "agent-drawer__suffix";
        title.textContent = agent.title;
        item.appendChild(title);
      }

      return item;
    }

    // home-terminal-parity-2: one section per project, in the order its
    // first agent was seen in the feed (`GET /api/agents` already walks
    // projects in a stable order, unassigned panes last), each section's own
    // rows sorted blocked before working before the rest.
    //
    // home-terminal-header-2: this is now the only shape. The project page
    // used to group its rows under status headings instead, so the same
    // cross-project switcher rearranged itself depending on which page you
    // happened to open it from — and the thing a reader is looking for in it
    // is which project an agent belongs to, which the status shape buried in
    // each row's suffix. One shape, both pages. What still differs between
    // the two instances is only where a row leads (`agentRow`).
    function renderByProject(agents) {
      var order = [];
      var groups = {};
      agents.forEach(function (agent) {
        var key = agent.project_name;
        if (!groups[key]) {
          groups[key] = [];
          order.push(key);
        }
        groups[key].push(agent);
      });
      order.forEach(function (project_name) {
        var rows = groups[project_name].slice().sort(function (a, b) {
          return statusRank(a.status) - statusRank(b.status);
        });
        var heading = document.createElement("div");
        heading.className = "agent-drawer__section";
        heading.textContent = project_name;
        list.appendChild(heading);
        rows.forEach(function (agent) {
          list.appendChild(agentRow(agent));
        });
      });
    }

    function render(agents) {
      list.textContent = "";
      if (!agents.length) {
        var empty = document.createElement("p");
        empty.className = "fg-empty";
        empty.textContent = "No agents";
        list.appendChild(empty);
        return;
      }
      renderByProject(agents);
    }

    function fetchAgents() {
      fetch("/api/agents", { credentials: "same-origin" })
        .then(function (res) {
          return res.ok ? res.json() : [];
        })
        .then(function (agents) {
          render(Array.isArray(agents) ? agents : []);
        })
        .catch(function () {});
    }

    function stop() {
      if (timer) {
        clearInterval(timer);
        timer = null;
      }
    }

    function start() {
      fetchAgents();
      stop();
      timer = setInterval(fetchAgents, POLL_MS);
    }

    toggle.addEventListener("change", function () {
      if (toggle.checked) start();
      else stop();
    });

    // The generic `.js-menu` handler above closes this drawer the same way
    // it closes any other menu — setting `toggle.checked = false` directly
    // on an outside click or Escape — which fires no "change" event of its
    // own. Watching the same two triggers here, deferred one tick so that
    // handler (registered earlier in this file, so it always runs first) has
    // already flipped the box, is what notices the drawer closed and stops
    // the poll.
    document.addEventListener("click", function () {
      setTimeout(function () {
        if (!toggle.checked) stop();
      }, 0);
    });
    document.addEventListener("keydown", function (e) {
      if (e.key !== "Escape") return;
      setTimeout(function () {
        if (!toggle.checked) stop();
      }, 0);
    });
  })();
})();
