// A small browser-like environment for scripts that expect a web page (BotGuard, the YouTube player).
// Interfaces, members and default values come from __DOM_SURFACE (generated); the behaviour is a minimal node tree.
(function (global) {
  "use strict";
  var surface = global.__DOM_SURFACE;
  var noop = function () {};
  var define = function (obj, key, value) { Object.defineProperty(obj, key, { value: value, configurable: true, writable: true }); };

  // ---- timers, driven by the host through __tick() ----
  var queue = [], timerId = 0;
  function schedule(fn, ms, every, args) {
    var id = ++timerId;
    queue.push({ id: id, fn: fn, at: Date.now() + (+ms || 0), every: every ? Math.max(+ms || 0, 1) : 0, args: args });
    return id;
  }
  global.setTimeout = function (fn, ms) { return schedule(fn, ms, false, Array.prototype.slice.call(arguments, 2)); };
  global.setInterval = function (fn, ms) { return schedule(fn, ms, true, Array.prototype.slice.call(arguments, 2)); };
  global.clearTimeout = global.clearInterval = function (id) { queue = queue.filter(function (t) { return t.id !== id; }); };
  global.requestAnimationFrame = function (fn) { return schedule(function () { fn(performance.now()); }, 16, false, []); };
  global.cancelAnimationFrame = global.clearTimeout;
  global.__tick = function () {
    var now = Date.now();
    var due = queue.filter(function (t) { return t.at <= now; });
    for (var i = 0; i < due.length; i++) {
      var t = due[i];
      if (t.every) t.at = now + t.every; else queue = queue.filter(function (x) { return x !== t; });
      try { if (typeof t.fn === "function") t.fn.apply(global, t.args); } catch (e) { global.__lastTimerError = String(e); }
    }
    return queue.length;
  };

  // ---- base64 ----
  var B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  global.atob = function (input) {
    var s = String(input).replace(/[\t\n\f\r ]+/g, "");
    if (s.length % 4 === 0) s = s.replace(/==?$/, "");
    if (s.length % 4 === 1 || /[^A-Za-z0-9+/]/.test(s)) throw new Error("The string to be decoded is not correctly encoded.");
    var out = "", buffer = 0, bits = 0;
    for (var i = 0; i < s.length; i++) {
      buffer = (buffer << 6) | B64.indexOf(s.charAt(i)); bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((buffer >> bits) & 255); }
    }
    return out;
  };
  global.btoa = function (input) {
    var s = String(input), out = "";
    for (var i = 0; i < s.length; i += 3) {
      var a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
      if (a > 255 || b > 255 || c > 255) throw new Error("The string to be encoded contains characters outside of the Latin1 range.");
      var n = (a << 16) | ((b || 0) << 8) | (c || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < s.length ? B64[n & 63] : "=");
    }
    return out;
  };

  // ---- interfaces from the surface ----
  var classes = {};
  function listen(type, fn, options) {
    if (options && typeof options === "object") { void options.capture; void options.once; void options.passive; void options.signal; }
  }
  function makeInterface(name, parent, members, constants) {
    var C = function () { throw new TypeError("Illegal constructor"); };
    Object.defineProperty(C, "name", { value: name });
    C.prototype = Object.create(parent ? parent.prototype : Object.prototype);
    define(C.prototype, "constructor", C);
    Object.defineProperty(C.prototype, Symbol.toStringTag, { value: name, configurable: true });
    Object.keys(constants || {}).forEach(function (k) { Object.defineProperty(C, k, { value: constants[k] }); Object.defineProperty(C.prototype, k, { value: constants[k] }); });
    Object.keys(members || {}).forEach(function (key) {
      var m = members[key];
      if (Object.prototype.hasOwnProperty.call(C.prototype, key)) return;
      if (m.t === "method") { define(C.prototype, key, function () { return undefined; }); return; }
      var initial = m.t === "string" || m.t === "number" || m.t === "boolean" ? m.v : m.t === "null" ? null : m.t === "object" ? undefined : undefined;
      Object.defineProperty(C.prototype, key, {
        get: function () {
          var f = this.__f;
          if (f && Object.prototype.hasOwnProperty.call(f, key)) return f[key];
          if (m.t === "object" && f) return (f[key] = makeObject(m.tag));
          return initial;
        },
        set: m.setter === false ? undefined : function (v) { fields(this)[key] = v; },
        configurable: true, enumerable: true,
      });
    });
    global[name] = C;
    classes[name] = C;
    return C;
  }
  function ensure(name) {
    if (!name) return null;
    if (classes[name]) return classes[name];
    var spec = surface.interfaces[name];
    if (!spec) return makeInterface(name, classes.HTMLElement || null, {}, {});
    return makeInterface(name, ensure(spec.parent), spec.members, spec.constants);
  }
  Object.keys(surface.interfaces).forEach(ensure);
  if (!classes.HTMLDocument) makeInterface("HTMLDocument", classes.Document, {}, {});
  if (!classes.HTMLUnknownElement) makeInterface("HTMLUnknownElement", classes.HTMLElement, {}, {});
  function fields(obj) { if (!obj.__f) Object.defineProperty(obj, "__f", { value: {} }); return obj.__f; }
  function instance(name) { var o = Object.create(ensure(name).prototype); fields(o); return o; }
  function makeObject(tag) {
    if (tag === "CSSStyleDeclaration") return makeStyle();
    if (classes[tag]) return instance(tag);
    var o = {};
    Object.defineProperty(o, Symbol.toStringTag, { value: tag, configurable: true });
    return o;
  }

  // ---- styles ----
  var CSS = makeInterface("CSSStyleDeclaration", null, {}, {});
  (surface.css || []).forEach(function (name) {
    var css = name === "cssFloat" ? "float" : name.replace(/[A-Z]/g, function (c) { return "-" + c.toLowerCase(); });
    Object.defineProperty(CSS.prototype, name, { get: function () { return this.__v[css] || ""; }, set: function (v) { this.__v[css] = String(v); }, configurable: true, enumerable: true });
  });
  CSS.prototype.getPropertyValue = function (name) { return this.__v[name] || ""; };
  CSS.prototype.getPropertyPriority = function () { return ""; };
  CSS.prototype.setProperty = function (name, value) { this.__v[name] = String(value); };
  CSS.prototype.removeProperty = function (name) { var old = this.__v[name] || ""; delete this.__v[name]; return old; };
  CSS.prototype.item = function (i) { return Object.keys(this.__v)[i] || ""; };
  Object.defineProperty(CSS.prototype, "length", { get: function () { return Object.keys(this.__v).length; }, configurable: true });
  Object.defineProperty(CSS.prototype, "cssText", {
    get: function () { var v = this.__v; return Object.keys(v).map(function (k) { return k + ": " + v[k] + ";"; }).join(" "); },
    set: function () {}, configurable: true,
  });
  function makeStyle() { var s = Object.create(CSS.prototype); Object.defineProperty(s, "__v", { value: {} }); return s; }

  // ---- collections ----
  function list(kind, items) {
    var l = instance(kind);
    items.forEach(function (item, i) { define(l, i, item); });
    fields(l).length = items.length;
    define(l, "item", function (i) { return items[i] || null; });
    define(l, Symbol.iterator, function () { return items[Symbol.iterator](); });
    define(l, "forEach", function (fn, self) { items.forEach(fn, self); });
    return l;
  }

  // ---- node tree ----
  var EventTarget = classes.EventTarget, Node = classes.Node, Element = classes.Element, Document = classes.Document;
  EventTarget.prototype.addEventListener = listen;
  EventTarget.prototype.removeEventListener = function (type, fn, options) { if (options && typeof options === "object") void options.capture; };
  EventTarget.prototype.dispatchEvent = function () { return true; };
  function kids(node) { var f = fields(node); return f.__kids || (f.__kids = []); }
  function connected(node) { for (var n = node; n; n = fields(n).parentNode) if (n === global.document) return true; return false; }
  function attach(parent, child) {
    if (!child || typeof child !== "object") throw new TypeError("parameter 1 is not of type 'Node'.");
    var old = fields(child).parentNode;
    if (old) detach(old, child);
    kids(parent).push(child);
    fields(child).parentNode = parent;
    refresh(child);
    return child;
  }
  function detach(parent, child) {
    var list = kids(parent), i = list.indexOf(child);
    if (i < 0) throw new Error("The node to be removed is not a child of this node.");
    list.splice(i, 1);
    fields(child).parentNode = null;
    refresh(child);
    return child;
  }
  function refresh(node) {
    var f = fields(node);
    if (f.__realm !== undefined) f.contentWindow = connected(node) ? f.__realm : null;
    kids(node).forEach(refresh);
  }
  function descendants(node, out) { kids(node).forEach(function (c) { out.push(c); descendants(c, out); }); return out; }
  function byTag(node, tag) {
    tag = String(tag).toLowerCase();
    return list("HTMLCollection", descendants(node, []).filter(function (n) { return fields(n).nodeType === 1 && (tag === "*" || fields(n).localName === tag); }));
  }
  Node.prototype.appendChild = function (child) { return attach(this, child); };
  Node.prototype.removeChild = function (child) { return detach(this, child); };
  Node.prototype.insertBefore = function (child, ref) {
    attach(this, child);
    var l = kids(this), i = ref ? l.indexOf(ref) : -1;
    if (i >= 0) { l.splice(l.indexOf(child), 1); l.splice(i, 0, child); }
    return child;
  };
  Node.prototype.contains = function (other) { return other === this || descendants(this, []).indexOf(other) >= 0; };
  Node.prototype.hasChildNodes = function () { return kids(this).length > 0; };
  Node.prototype.cloneNode = function () { return global.document.createElement(this.localName || "div"); };
  Node.prototype.getRootNode = function () { var n = this; while (fields(n).parentNode) n = fields(n).parentNode; return n; };
  ["parentNode", "parentElement"].forEach(function (key) {
    Object.defineProperty(Node.prototype, key, { get: function () { return fields(this).parentNode || null; }, configurable: true });
  });
  Object.defineProperty(Node.prototype, "isConnected", { get: function () { return connected(this); }, configurable: true });
  Object.defineProperty(Node.prototype, "childNodes", { get: function () { return list("NodeList", kids(this).slice()); }, configurable: true });
  Object.defineProperty(Node.prototype, "firstChild", { get: function () { return kids(this)[0] || null; }, configurable: true });
  Object.defineProperty(Node.prototype, "lastChild", { get: function () { var l = kids(this); return l[l.length - 1] || null; }, configurable: true });
  Object.defineProperty(Node.prototype, "nextSibling", { get: function () { var p = fields(this).parentNode; if (!p) return null; var l = kids(p); return l[l.indexOf(this) + 1] || null; }, configurable: true });
  Object.defineProperty(Node.prototype, "previousSibling", { get: function () { var p = fields(this).parentNode; if (!p) return null; var l = kids(p); return l[l.indexOf(this) - 1] || null; }, configurable: true });
  Object.defineProperty(Node.prototype, "textContent", {
    get: function () { var f = fields(this); return f.nodeType === 9 ? null : f.__text !== undefined ? f.__text : kids(this).map(function (c) { return c.textContent || ""; }).join(""); },
    set: function (v) { fields(this).__text = String(v); }, configurable: true,
  });
  Object.defineProperty(Element.prototype, "children", { get: function () { return list("HTMLCollection", kids(this).filter(function (n) { return fields(n).nodeType === 1; })); }, configurable: true });
  Object.defineProperty(Element.prototype, "childElementCount", { get: function () { return this.children.length; }, configurable: true });
  Object.defineProperty(Element.prototype, "firstElementChild", { get: function () { return this.children[0] || null; }, configurable: true });
  Element.prototype.getElementsByTagName = function (tag) { return byTag(this, tag); };
  Element.prototype.querySelectorAll = function (s) { return /^[a-z0-9]+$/i.test(s) ? list("NodeList", Array.prototype.slice.call(byTag(this, s))) : list("NodeList", []); };
  Element.prototype.querySelector = function (s) { return this.querySelectorAll(s)[0] || null; };
  Element.prototype.getAttribute = function (name) { var a = fields(this).__attrs || {}; return Object.prototype.hasOwnProperty.call(a, name) ? a[name] : null; };
  Element.prototype.setAttribute = function (name, value) { var f = fields(this); (f.__attrs || (f.__attrs = {}))[String(name).toLowerCase()] = String(value); };
  Element.prototype.hasAttribute = function (name) { return this.getAttribute(name) !== null; };
  Element.prototype.removeAttribute = function (name) { var a = fields(this).__attrs; if (a) delete a[name]; };
  Element.prototype.getAttributeNames = function () { return Object.keys(fields(this).__attrs || {}); };
  Element.prototype.getBoundingClientRect = function () { return { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0, toJSON: noop }; };
  Element.prototype.getClientRects = function () { return []; };
  Element.prototype.matches = function () { return false; };
  Element.prototype.closest = function () { return null; };
  Object.defineProperty(classes.HTMLElement.prototype, "style", { get: function () { var f = fields(this); return f.__style || (f.__style = makeStyle()); }, set: noop, configurable: true });

  function element(tag, doc) {
    var lower = String(tag).toLowerCase();
    var name = surface.tags[lower] || "HTMLUnknownElement";
    var el = instance(name);
    var f = fields(el);
    f.nodeType = 1; f.localName = lower; f.tagName = f.nodeName = lower.toUpperCase();
    f.namespaceURI = "http://www.w3.org/1999/xhtml"; f.ownerDocument = doc; f.parentNode = null;
    if (lower === "iframe" || lower === "frame") {
      f.__realm = (global.__realms && global.__realms.length) ? global.__realms.shift() : global;
      f.contentWindow = null; f.contentDocument = null;
    }
    return el;
  }

  // ---- events ----
  function event(kind) {
    var name = surface.events[kind] || "Event";
    var e = instance(name);
    var init = function (type, bubbles, cancelable) { var f = fields(e); f.type = String(type); f.bubbles = !!bubbles; f.cancelable = !!cancelable; };
    ["initEvent", "initUIEvent", "initMouseEvent", "initKeyboardEvent"].forEach(function (m) { define(e, m, init); });
    define(e, "preventDefault", function () { fields(e).defaultPrevented = true; });
    define(e, "stopPropagation", noop);
    define(e, "stopImmediatePropagation", noop);
    fields(e).timeStamp = performance.now();
    return e;
  }

  // ---- the window ----
  var WindowCtor = function Window() { throw new TypeError("Illegal constructor"); };
  WindowCtor.prototype = Object.create(EventTarget.prototype);
  define(WindowCtor.prototype, "constructor", WindowCtor);
  Object.defineProperty(WindowCtor.prototype, Symbol.toStringTag, { value: "Window", configurable: true });
  global.Window = WindowCtor;
  try { Object.setPrototypeOf(global, WindowCtor.prototype); } catch (e) {}
  global.window = global.self = global.top = global.parent = global.frames = global;
  global.length = 0; global.name = ""; global.closed = false; global.opener = null; global.frameElement = null;
  global.origin = "https://music.youtube.com"; global.isSecureContext = true;
  global.innerWidth = 1920; global.innerHeight = 969; global.outerWidth = 1920; global.outerHeight = 1040;
  global.devicePixelRatio = 1; global.scrollX = global.scrollY = global.pageXOffset = global.pageYOffset = 0;
  global.screenX = global.screenY = global.screenLeft = global.screenTop = 0;

  var hostNow = typeof global.__hostNow === "function" ? global.__hostNow : function () { return Date.now(); };
  var origin = Date.now() - hostNow();
  global.performance = { now: function () { return hostNow(); }, timeOrigin: origin, timing: { navigationStart: Math.floor(origin) },
    mark: noop, measure: noop, clearMarks: noop, clearMeasures: noop, getEntries: function () { return []; },
    getEntriesByType: function () { return []; }, getEntriesByName: function () { return []; }, toJSON: function () { return {}; } };
  Object.defineProperty(global.performance, Symbol.toStringTag, { value: "Performance", configurable: true });

  var href = global.__pageUrl || "https://music.youtube.com/";
  var url = /^(https?:)\/\/([^/:]+)(:\d+)?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/.exec(href) || [];
  global.location = { href: href, protocol: url[1] || "https:", hostname: url[2] || "", host: (url[2] || "") + (url[3] || ""), port: (url[3] || "").slice(1),
    pathname: url[4] || "/", search: url[5] || "", hash: url[6] || "", origin: (url[1] || "https:") + "//" + (url[2] || "") + (url[3] || ""),
    assign: noop, replace: noop, reload: noop, toString: function () { return href; } };
  Object.defineProperty(global.location, Symbol.toStringTag, { value: "Location", configurable: true });
  var ua = global.__userAgent || "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
  global.navigator = { userAgent: ua, appVersion: ua.replace(/^Mozilla\//, ""), appName: "Netscape", appCodeName: "Mozilla", product: "Gecko",
    productSub: "20030107", vendor: "Google Inc.", vendorSub: "", platform: "Win32", language: "en-US", languages: ["en-US", "en"],
    cookieEnabled: true, onLine: true, doNotTrack: null, hardwareConcurrency: 8, maxTouchPoints: 0, pdfViewerEnabled: true, webdriver: false,
    plugins: [], mimeTypes: [], javaEnabled: function () { return false; }, sendBeacon: function () { return true; } };
  Object.defineProperty(global.navigator, Symbol.toStringTag, { value: "Navigator", configurable: true });
  global.screen = { width: 1920, height: 1080, availWidth: 1920, availHeight: 1040, availLeft: 0, availTop: 0, colorDepth: 24, pixelDepth: 24,
    orientation: { type: "landscape-primary", angle: 0 } };
  Object.defineProperty(global.screen, Symbol.toStringTag, { value: "Screen", configurable: true });
  global.history = { length: 1, state: null, scrollRestoration: "auto", pushState: noop, replaceState: noop, back: noop, forward: noop, go: noop };
  function storage() {
    var data = {};
    return { getItem: function (k) { return Object.prototype.hasOwnProperty.call(data, k) ? data[k] : null; }, setItem: function (k, v) { data[k] = String(v); },
      removeItem: function (k) { delete data[k]; }, clear: function () { data = {}; }, key: function (i) { return Object.keys(data)[i] || null; },
      get length() { return Object.keys(data).length; } };
  }
  global.localStorage = storage(); global.sessionStorage = storage();
  global.matchMedia = function (q) { return { matches: false, media: String(q), onchange: null, addListener: noop, removeListener: noop, addEventListener: listen, removeEventListener: noop }; };
  global.getComputedStyle = function (el) { return el && el.style ? el.style : makeStyle(); };
  global.MutationObserver = function () { this.observe = noop; this.disconnect = noop; this.takeRecords = function () { return []; }; };
  global.ResizeObserver = global.IntersectionObserver = function () { this.observe = noop; this.unobserve = noop; this.disconnect = noop; };
  global.XMLHttpRequest = function () { this.readyState = 0; this.open = noop; this.send = noop; this.setRequestHeader = noop; this.abort = noop; };
  global.Image = function (w, h) { var img = global.document.createElement("img"); if (w !== undefined) img.width = w; if (h !== undefined) img.height = h; return img; };
  global.postMessage = noop; global.focus = noop; global.blur = noop; global.scrollTo = global.scroll = global.scrollBy = noop;
  global.alert = noop; global.confirm = function () { return false; }; global.prompt = function () { return null; };
  global.queueMicrotask = global.queueMicrotask || function (fn) { Promise.resolve().then(fn); };

  // ---- the document ----
  var doc = instance("HTMLDocument");
  var df = fields(doc);
  df.nodeType = 9; df.nodeName = "#document"; df.ownerDocument = null; df.parentNode = null;
  df.readyState = "complete"; df.compatMode = "CSS1Compat"; df.characterSet = df.charset = df.inputEncoding = "UTF-8";
  df.contentType = "text/html"; df.URL = df.documentURI = href; df.domain = global.location.hostname; df.referrer = ""; df.cookie = "";
  df.title = ""; df.visibilityState = "visible"; df.hidden = false; df.defaultView = global; df.designMode = "off"; df.dir = "";
  df.lastModified = new Date().toLocaleString();
  global.document = doc;
  var html = element("html", doc), head = element("head", doc), body = element("body", doc);
  attach(doc, html); attach(html, head); attach(html, body);
  df.documentElement = html; df.head = head; df.body = body; df.scrollingElement = html; df.activeElement = body;
  doc.createElement = function (tag) { return element(tag, doc); };
  doc.createElementNS = function (ns, tag) { var el = element(tag, doc); fields(el).namespaceURI = ns; return el; };
  doc.createTextNode = function (text) { var t = instance("Text"); var f = fields(t); f.nodeType = 3; f.nodeName = "#text"; f.data = f.nodeValue = String(text); f.__text = String(text); f.ownerDocument = doc; return t; };
  doc.createComment = function (text) { var t = instance("Comment"); var f = fields(t); f.nodeType = 8; f.nodeName = "#comment"; f.data = f.nodeValue = String(text); f.ownerDocument = doc; return t; };
  doc.createDocumentFragment = function () { var t = instance("DocumentFragment"); var f = fields(t); f.nodeType = 11; f.nodeName = "#document-fragment"; f.ownerDocument = doc; return t; };
  doc.createEvent = function (kind) { return event(kind); };
  doc.getElementsByTagName = function (tag) { return byTag(doc, tag); };
  doc.getElementsByClassName = function () { return list("HTMLCollection", []); };
  doc.getElementsByName = function () { return list("NodeList", []); };
  doc.getElementById = function () { return null; };
  doc.querySelectorAll = function (s) { return /^[a-z0-9]+$/i.test(s) ? list("NodeList", Array.prototype.slice.call(byTag(doc, s))) : list("NodeList", []); };
  doc.querySelector = function (s) { return doc.querySelectorAll(s)[0] || null; };
  doc.hasFocus = function () { return true; };
  doc.addEventListener = listen;
  global.Event = function (type, init) { var e = event("Event"); fields(e).type = String(type); fields(e).bubbles = !!(init && init.bubbles); return e; };
  global.CustomEvent = function (type, init) { var e = event("Event"); fields(e).type = String(type); fields(e).detail = init ? init.detail : null; return e; };
})(globalThis);
