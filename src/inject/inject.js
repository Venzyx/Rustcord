// Rustcord page integration. Runs at document creation in the main frame of
// the official Discord web client. Kept dependency-free and defensive: every
// hook must fail closed so Discord keeps working even if a selector changes.
(() => {
  'use strict';
  if (window.__rustcord) return;
  if (!/(^|\.)discord\.com$/.test(location.hostname)) return;

  const invoke = (cmd, args) => {
    const ipc = window.__TAURI_INTERNALS__;
    return ipc ? ipc.invoke(cmd, args || {}) : Promise.reject(new Error('ipc unavailable'));
  };
  const rc = (window.__rustcord = Object.freeze({ version: '__RC_VERSION__' }));
  void rc;

  // ---------------------------------------------------------------- module patches
  // A few desktop features exist in the web bundle but are hidden behind a
  // "is this the desktop app?" check. We flip only those specific checks by
  // rewriting the module factory before it first runs. Each patch is matched
  // by a unique source marker and fails safe (logged, original kept).
  const PATCHES = [
    {
      // Stream menu → "Stream Quality" (resolution + FPS). Discord's own
      // Nitro/boost rules and upsells still apply to every option.
      name: 'stream-quality-menu',
      find: 'id:"stream-settings"',
      match: /\b[\w$]+\.isPlatformEmbedded\?(\(0,[\w$]+\.jsx\)\([\w$]+\.Dr,\{id:"stream-settings")/,
      replace: 'true?$1',
    },
  ];
  const patchStatus = Object.fromEntries(PATCHES.map((p) => [p.name, 'pending']));
  Object.defineProperty(window, '__rustcordPatches', { get: () => ({ ...patchStatus }) });

  function patchFactory(id, factory) {
    if (typeof factory !== 'function') return factory;
    const pending = PATCHES.filter((p) => patchStatus[p.name] === 'pending');
    if (!pending.length) return factory;
    let src;
    try { src = Function.prototype.toString.call(factory); } catch { return factory; }
    let out = src;
    for (const p of pending) {
      if (!src.includes(p.find)) continue;
      const next = out.replace(p.match, p.replace);
      if (next === out) {
        patchStatus[p.name] = 'no-match';
        console.warn('[rustcord] patch did not match:', p.name);
      } else {
        out = next;
        patchStatus[p.name] = 'applied';
      }
    }
    if (out === src) return factory;
    // Method shorthand ("600125(e,t,n){…}") is not a valid expression.
    const code = out.replace(/^[\w$]+\s*\(/, 'function(');
    try {
      return (0, eval)(`(${code})\n//# sourceURL=rustcord-patched-${String(id).replace(/\W/g, '')}.js`);
    } catch (e) {
      for (const p of pending) if (patchStatus[p.name] === 'applied') patchStatus[p.name] = 'failed';
      console.error('[rustcord] patch failed for module', id, e);
      return factory;
    }
  }

  function patchModules(modules) {
    for (const id of Object.keys(modules)) {
      if (!PATCHES.some((p) => patchStatus[p.name] === 'pending')) return;
      const f = modules[id];
      const patched = patchFactory(id, f);
      if (patched !== f) modules[id] = patched;
    }
  }

  // The webpack/rspack runtime does `require.m = modules` before running any
  // module. Catch that assignment, patch in place, and keep watching for
  // modules added later by lazily loaded chunks.
  try {
    Object.defineProperty(Function.prototype, 'm', {
      configurable: true,
      enumerable: false,
      get() { return undefined; },
      set(modules) {
        let value = modules;
        if (modules && typeof modules === 'object') {
          try {
            patchModules(modules);
            value = new Proxy(modules, {
              set(target, key, factory) {
                target[key] = patchFactory(key, factory);
                return true;
              },
            });
          } catch (e) {
            console.error('[rustcord] module hook', e);
          }
        }
        Object.defineProperty(this, 'm', { value, writable: true, configurable: true, enumerable: true });
      },
    });
  } catch (e) {
    console.error('[rustcord] could not install module hook', e);
  }

  // ---------------------------------------------------------------- stream frame rate
  // The web media engine always captures the screen at 30 fps. Capture at the
  // frame rate chosen in Stream Quality instead, and apply changes live.
  const streamQuality = (() => {
    let fps = null;
    const tracks = new Set();
    const md = navigator.mediaDevices;
    const original = md && md.getDisplayMedia;
    if (original) {
      md.getDisplayMedia = async function (constraints) {
        let c = constraints;
        if (fps && c && c.video && typeof c.video === 'object') c = { ...c, video: { ...c.video, frameRate: fps } };
        const stream = await original.call(this, c);
        for (const t of stream.getVideoTracks()) {
          tracks.add(t);
          t.addEventListener('ended', () => tracks.delete(t), { once: true });
        }
        return stream;
      };
    }
    function setFps(next) {
      if (!Number.isFinite(next) || next <= 0 || next === fps) return;
      fps = next;
      for (const t of tracks) {
        if (t.readyState !== 'live') { tracks.delete(t); continue; }
        t.applyConstraints({ ...t.getConstraints(), frameRate: fps }).catch(() => {});
      }
    }
    function attach(dispatcher) {
      try {
        dispatcher.subscribe('MEDIA_ENGINE_SET_GO_LIVE_SOURCE', (a) => setFps(a?.settings?.qualityOptions?.frameRate));
        dispatcher.subscribe('STREAM_UPDATE_SETTINGS', (a) => setFps(a?.frameRate));
      } catch (e) {
        console.warn('[rustcord] stream quality hook', e);
      }
    }
    return { attach };
  })();

  // ---------------------------------------------------------------- notifications
  // Discord's web client uses the standard Notification API. We route it to
  // native Windows toasts; Discord plays its own sound, so toasts are silent.
  const live = new Map();
  let seq = 0;

  const toBase64 = (buf) => {
    const bytes = new Uint8Array(buf);
    let s = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return btoa(s);
  };

  const ICON_HOSTS = /^(cdn\.discordapp\.com|media\.discordapp\.net|([a-z0-9-]+\.)?discord\.com)$/;
  async function iconPng(url) {
    if (!url) return null;
    try {
      const u = new URL(url, location.href);
      if (u.protocol !== 'https:' || !ICON_HOSTS.test(u.hostname)) return null;
      if (u.hostname === 'cdn.discordapp.com') u.searchParams.set('size', '128');
      const res = await fetch(u.href, { credentials: 'omit', cache: 'force-cache' });
      if (!res.ok) return null;
      const blob = await res.blob();
      if (blob.size > 2 * 1024 * 1024) return null;
      const bmp = await createImageBitmap(blob, { resizeWidth: 96, resizeHeight: 96, resizeQuality: 'high' });
      const canvas = new OffscreenCanvas(96, 96);
      canvas.getContext('2d').drawImage(bmp, 0, 0);
      bmp.close();
      const png = await canvas.convertToBlob({ type: 'image/png' });
      return toBase64(await png.arrayBuffer());
    } catch {
      return null;
    }
  }

  class RcNotification extends EventTarget {
    constructor(title, options) {
      super();
      const o = options || {};
      this.title = String(title ?? '');
      this.body = String(o.body ?? '');
      this.icon = o.icon ? String(o.icon) : '';
      this.tag = o.tag ? String(o.tag) : '';
      this.data = o.data ?? null;
      this.silent = !!o.silent;
      this.onclick = this.onclose = this.onerror = this.onshow = null;
      this.__id = ++seq;
      live.set(this.__id, this);
      if (live.size > 100) live.delete(live.keys().next().value);
      iconPng(this.icon)
        .then((icon) => invoke('rc_notify', { id: this.__id, title: this.title, body: this.body, icon }))
        .then(() => this.__fire('show'), () => this.__fire('error'));
    }
    __fire(type) {
      const ev = new Event(type, { cancelable: true });
      const handler = this['on' + type];
      try { if (typeof handler === 'function') handler.call(this, ev); } catch (e) { console.error(e); }
      this.dispatchEvent(ev);
    }
    close() {
      if (live.delete(this.__id)) this.__fire('close');
    }
    static get permission() { return 'granted'; }
    static requestPermission(cb) {
      if (typeof cb === 'function') cb('granted');
      return Promise.resolve('granted');
    }
    static get maxActions() { return 0; }
  }
  Object.defineProperty(window, 'Notification', { value: RcNotification, writable: true, configurable: true });

  // Called from Rust when a toast is clicked (window is already focused).
  Object.defineProperty(window, '__rcNotificationActivated', {
    value: (id) => {
      const n = live.get(id);
      if (!n) return;
      live.delete(id);
      n.__fire('click');
    },
  });

  // ---------------------------------------------------------------- rich presence bridge
  // Rust runs the local RPC server and game detection; activities arrive here
  // and are handed to Discord's own Flux dispatcher as LOCAL_ACTIVITY_UPDATE,
  // normalized exactly like the desktop client's SET_ACTIVITY handler does.
  const presence = (() => {
    let dispatcher = null;
    let fetchAssetIds = null;
    let userStore = null;
    let ready = false;
    let reportedUser = '';
    const queue = [];
    const appNames = new Map();

    // Some exports are proxies that claim every property; skip those.
    const isReal = (v) => {
      try { return v != null && v['__rc_probe_' + Math.random().toString(36).slice(2)] === undefined; } catch { return false; }
    };

    function runtimes() {
      const reqs = [];
      const chunk = window.webpackChunkdiscord_app;
      if (!chunk || typeof chunk.push !== 'function') return reqs;
      try {
        chunk.push([[Symbol('rustcord')], {}, (r) => { reqs.push(r); }]);
        chunk.pop();
      } catch { /* runtime not ready */ }
      return reqs;
    }

    function scan() {
      for (const req of runtimes()) {
        const cache = req.c;
        if (!cache) continue;
        for (const id in cache) {
          const ex = cache[id] && cache[id].exports;
          if (!ex || (typeof ex !== 'object' && typeof ex !== 'function') || !isReal(ex)) continue;
          let values;
          try { values = Object.values(ex); } catch { continue; }
          for (const v of values) {
            if (!v || (typeof v !== 'object' && typeof v !== 'function') || !isReal(v)) continue;
            try {
              if (!dispatcher && typeof v.dispatch === 'function' && typeof v.subscribe === 'function'
                  && v._actionHandlers && typeof v._actionHandlers === 'object') {
                dispatcher = v;
              } else if (!userStore && typeof v.getCurrentUser === 'function' && typeof v.getUser === 'function') {
                userStore = v;
              } else if (!fetchAssetIds && typeof v === 'function' && v.length >= 2) {
                const src = Function.prototype.toString.call(v);
                if (src.length < 4000 && src.includes('APPLICATION_ASSETS_FETCH')) fetchAssetIds = v;
              }
            } catch { /* ignore hostile getters */ }
          }
          if (dispatcher && userStore && fetchAssetIds) return;
        }
      }
    }

    function currentUser() {
      try {
        const u = userStore && userStore.getCurrentUser();
        if (!u || !u.id) return null;
        return { id: String(u.id), username: String(u.username || ''), global_name: u.globalName ?? u.global_name ?? null,
          avatar: u.avatar ?? null, discriminator: u.discriminator ?? '0' };
      } catch { return null; }
    }

    async function appName(id) {
      if (appNames.has(id)) return appNames.get(id);
      let name = null;
      try {
        const res = await fetch(`/api/v9/oauth2/applications/${id}/rpc`, { credentials: 'omit' });
        if (res.ok) name = (await res.json()).name || null;
      } catch { /* offline */ }
      appNames.set(id, name);
      return name;
    }

    async function normalize(input) {
      const a = JSON.parse(JSON.stringify(input));
      const appId = a.application_id ? String(a.application_id) : undefined;
      if (!a.name && appId) a.name = (await appName(appId)) || 'Game';
      if (a.type == null) a.type = 0;
      a.platform = 'desktop';
      const instance = !!a.instance;
      delete a.instance;
      const partyPrivacy = a.party && a.party.privacy;
      if (a.party) delete a.party.privacy;
      if (instance) a.flags = (a.flags || 0) | 1;
      const metadata = {};
      if (Array.isArray(a.buttons) && a.buttons.length) {
        const buttons = a.buttons.filter((b) => b && typeof b.label === 'string' && typeof b.url === 'string').slice(0, 2);
        metadata.button_urls = buttons.map((b) => b.url);
        a.buttons = buttons.map((b) => b.label);
      } else {
        delete a.buttons;
      }
      a.metadata = metadata;
      if (a.timestamps) {
        for (const k of ['start', 'end']) {
          const t = a.timestamps[k];
          if (t != null && Date.now().toString().length - String(Math.floor(t)).length > 2) a.timestamps[k] = Math.floor(t * 1000);
        }
      }
      if (a.assets && appId) {
        const keys = [a.assets.large_image, a.assets.small_image, a.assets.invite_cover_image];
        let ids = null;
        try { if (fetchAssetIds) ids = await fetchAssetIds(appId, keys); } catch { ids = null; }
        ['large_image', 'small_image', 'invite_cover_image'].forEach((k, i) => {
          const resolved = ids ? ids[i] : keys[i];
          if (resolved != null) a.assets[k] = resolved; else delete a.assets[k];
        });
      }
      return { activity: a, partyPrivacy };
    }

    async function apply(msg) {
      const socketId = String(msg.socketId);
      const pid = msg.pid ?? undefined;
      if (!msg.activity) {
        dispatcher.dispatch({ type: 'LOCAL_ACTIVITY_UPDATE', socketId, pid, activity: null });
        return;
      }
      const { activity, partyPrivacy } = await normalize(msg.activity);
      dispatcher.dispatch({
        type: 'LOCAL_ACTIVITY_UPDATE', socketId, pid,
        applicationId: activity.application_id, activity, partyPrivacy,
      });
    }

    let chain = Promise.resolve();
    function receive(msg) {
      if (!ready) { queue.push(msg); return; }
      // Serialize so a fast set→clear can't be reordered by asset lookups.
      chain = chain.then(() => apply(msg)).catch((e) => console.warn('[rustcord] activity', e));
    }

    async function loadDetectables() {
      try {
        const res = await fetch('/api/v9/applications/detectable', { credentials: 'omit' });
        if (!res.ok) return;
        const list = await res.json();
        const compact = [];
        for (const app of list) {
          const exes = (app.executables || [])
            .filter((e) => e && e.os === 'win32' && !e.is_launcher && typeof e.name === 'string')
            .map((e) => e.name);
          if (exes.length) compact.push({ id: String(app.id), name: String(app.name || ''), exes });
        }
        const n = await invoke('rc_rpc_detectables', { list: compact });
        console.info('[rustcord] game detection ready:', n, 'apps');
      } catch (e) {
        console.warn('[rustcord] detectables', e);
      }
    }

    function reportReady() {
      const user = currentUser();
      const key = user ? user.id : '';
      if (ready && key === reportedUser) return;
      reportedUser = key;
      invoke('rc_rpc_ready', { user }).catch(() => {});
    }

    let attempts = 0;
    let extraScans = 0;
    let detectablesLoaded = false;
    function boot() {
      if (!dispatcher) scan();
      if (dispatcher) {
        // Asset helper / user store may load lazily; a few bounded rescans.
        if ((!fetchAssetIds || !userStore) && extraScans < 10) { extraScans++; scan(); }
        const wasReady = ready;
        ready = true;
        reportReady();
        if (!wasReady) {
          streamQuality.attach(dispatcher);
          queue.splice(0).forEach(receive);
          if (!detectablesLoaded) { detectablesLoaded = true; setTimeout(loadDetectables, 5000); }
        }
      }
      attempts++;
      // Keep watching for login/logout (user changes) at a slower pace.
      setTimeout(boot, !dispatcher ? 1000 : attempts < 60 ? 2000 : 15000);
    }

    return { receive, boot };
  })();
  Object.defineProperty(window, '__rcActivity', { value: (msg) => presence.receive(msg) });
  if (document.readyState === 'complete') presence.boot();
  else addEventListener('load', () => presence.boot(), { once: true });

  // ---------------------------------------------------------------- deep links
  // Called from Rust for discord:// links. Routes inside the running app go
  // through Discord's own router (history + popstate), so nothing reloads.
  const SPA_ROUTES = /^\/(channels|store|shop|discovery|guild-discovery|quests|activities|library)(\/|$|\?)/;
  Object.defineProperty(window, '__rcNavigate', {
    value: (path) => {
      if (typeof path !== 'string' || !path.startsWith('/') || path.startsWith('//')) return;
      const inApp = SPA_ROUTES.test(location.pathname) || location.pathname === '/app';
      if (inApp && SPA_ROUTES.test(path)) {
        history.pushState(history.state, '', path);
        dispatchEvent(new PopStateEvent('popstate', { state: history.state }));
      } else {
        location.assign(path);
      }
    },
  });

  // ---------------------------------------------------------------- shortcuts
  addEventListener('keydown', (e) => {
    const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
    if (ctrl && !e.shiftKey && (e.key === 'r' || e.key === 'R')) {
      e.preventDefault();
      location.reload();
    } else if (ctrl && e.shiftKey && (e.key === 'I' || e.key === 'i')) {
      invoke('rc_window', { action: 'devtools' }).catch(() => {});
    }
  }, true);

  // ---------------------------------------------------------------- DOM-dependent parts
  const onReady = (fn) => (document.readyState === 'loading'
    ? document.addEventListener('DOMContentLoaded', fn, { once: true })
    : fn());

  onReady(() => {
    // ------------------------------------------------ styles
    const style = document.createElement('style');
    style.id = 'rc-style';
    style.textContent = `
      :root { --rc-bar-h: 32px; }
      #rc-winbtns { position: fixed; top: 0; right: 0; height: var(--rc-bar-h); display: flex;
        z-index: 2147483646; pointer-events: auto; }
      #rc-winbtns button { all: unset; box-sizing: border-box; width: 44px; height: 100%;
        display: flex; align-items: center; justify-content: center;
        color: var(--interactive-normal, var(--interactive-icon-default, #b5bac1)); }
      #rc-winbtns button:hover { background: var(--background-modifier-hover, rgba(78,80,88,.4));
        color: var(--interactive-hover, var(--interactive-icon-hover, #dbdee1)); }
      #rc-winbtns button.rc-close:hover { background: #da373c; color: #fff; }
      #rc-winbtns svg { width: 10px; height: 10px; pointer-events: none; }
      /* The web client advertises the desktop app; inside Rustcord it is redundant. */
      div[class*="listItem_"]:has(> * [data-list-item-id="guildsnav___app-download-button"]),
      div[class*="listItem_"]:has([data-list-item-id="guildsnav___app-download-button"]) { display: none !important; }
    `;
    document.head.appendChild(style);
    const dynamicRules = document.createElement('style');
    dynamicRules.id = 'rc-style-dyn';
    document.head.appendChild(dynamicRules);

    // ------------------------------------------------ window controls
    const svg = {
      min: '<svg viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" stroke-width="1"/></svg>',
      max: '<svg viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" fill="none" stroke="currentColor"/></svg>',
      restore: '<svg viewBox="0 0 10 10"><path d="M2.5 2.5V.5h7v7h-2" fill="none" stroke="currentColor"/><rect x=".5" y="2.5" width="7" height="7" fill="none" stroke="currentColor"/></svg>',
      close: '<svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1.1"/></svg>',
    };
    const bar = document.createElement('div');
    bar.id = 'rc-winbtns';
    bar.setAttribute('role', 'group');
    const mk = (cls, label, action, icon) => {
      const b = document.createElement('button');
      b.className = cls;
      b.setAttribute('aria-label', label);
      b.tabIndex = -1;
      b.innerHTML = icon;
      b.addEventListener('click', (e) => {
        e.stopPropagation();
        invoke('rc_window', { action }).then(syncMax, () => {});
      });
      bar.appendChild(b);
      return b;
    };
    mk('rc-min', 'Minimize', 'minimize', svg.min);
    const maxBtn = mk('rc-max', 'Maximize', 'toggle_maximize', svg.max);
    mk('rc-close', 'Close', 'close', svg.close);
    document.body.appendChild(bar);

    function syncMax() {
      invoke('rc_window', { action: 'is_maximized' }).then((m) => {
        maxBtn.innerHTML = m ? svg.restore : svg.max;
        maxBtn.setAttribute('aria-label', m ? 'Restore' : 'Maximize');
      }, () => {});
    }
    let resizeTimer = 0;
    addEventListener('resize', () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(syncMax, 80); });
    syncMax();

    // Find Discord's own top bar (a full-width strip at y=0) so the drag
    // region and controls match its height and its trailing icons are not
    // covered. Falls back to a 32px invisible strip (e.g. on the login page).
    let barHeight = 32;
    let barClass = '';
    function measure() {
      let found = null;
      for (const el of document.elementsFromPoint(innerWidth / 2, 2)) {
        if (el === bar || bar.contains(el) || el === document.documentElement || el === document.body) continue;
        const r = el.getBoundingClientRect();
        if (r.top <= 0 && r.height >= 20 && r.height <= 56 && r.width >= innerWidth * 0.6) found = el;
      }
      const h = found ? Math.round(found.getBoundingClientRect().height) : 32;
      const cls = found && found.classList.length ? found.classList[0] : '';
      if (h !== barHeight) {
        barHeight = h;
        document.documentElement.style.setProperty('--rc-bar-h', h + 'px');
      }
      if (cls !== barClass) {
        barClass = cls;
        dynamicRules.textContent = cls && /^[\w-]+$/.test(cls)
          ? `.${cls} { padding-right: ${3 * 44}px !important; box-sizing: border-box; }`
          : '';
      }
    }
    let measureQueued = false;
    const queueMeasure = () => {
      if (measureQueued) return;
      measureQueued = true;
      requestAnimationFrame(() => { measureQueued = false; measure(); });
    };
    new MutationObserver(queueMeasure).observe(document.body, { childList: true, subtree: false });
    addEventListener('resize', queueMeasure);
    setInterval(measure, 2000);
    measure();

    // Drag / double-click-to-maximize on the top strip, except on controls.
    const INTERACTIVE = 'button,a,input,textarea,select,[role="button"],[role="link"],[role="tab"],[role="menuitem"],[contenteditable=""],[contenteditable="true"],#rc-winbtns';
    addEventListener('mousedown', (e) => {
      if (e.button !== 0 || e.clientY > barHeight) return;
      if (e.target instanceof Element && e.target.closest(INTERACTIVE)) return;
      e.preventDefault();
      invoke('rc_window', { action: e.detail === 2 ? 'toggle_maximize' : 'drag' }).then(syncMax, () => {});
    }, true);

    // ------------------------------------------------ unread badge
    // Discord's tab title is "(N) Discord | …" for N mentions and "• Discord | …"
    // for unread messages without mentions.
    let lastBadge = '';
    const reportBadge = () => {
      const t = document.title || '';
      const m = /^\((\d+)\)/.exec(t);
      const mentions = m ? Math.min(parseInt(m[1], 10) || 0, 9999) : 0;
      const unread = !m && /^[••]/.test(t);
      const key = mentions + '|' + unread;
      if (key === lastBadge) return;
      lastBadge = key;
      invoke('rc_set_badge', { mentions, unread }).catch(() => {});
    };
    new MutationObserver(reportBadge).observe(document.head, { subtree: true, childList: true, characterData: true });
    reportBadge();
  });
})();
