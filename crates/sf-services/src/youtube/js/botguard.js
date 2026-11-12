var __error = "", __bgResponse = "", __signals = [], __minter = null, __token = "";

function __b64ToBytes(text) {
  var s = text.replace(/-/g, "+").replace(/_/g, "/").replace(/\./g, "=");
  var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  var out = [], buffer = 0, bits = 0;
  for (var i = 0; i < s.length; i++) {
    var c = chars.indexOf(s[i]);
    if (c < 0) continue;
    buffer = (buffer << 6) | c; bits += 6;
    if (bits >= 8) { bits -= 8; out.push((buffer >> bits) & 255); }
  }
  return new Uint8Array(out);
}
function __bytesToB64Url(bytes) {
  var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  var out = "";
  for (var i = 0; i < bytes.length; i += 3) {
    var n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
    out += chars[(n >> 18) & 63] + chars[(n >> 12) & 63];
    out += i + 1 < bytes.length ? chars[(n >> 6) & 63] : "=";
    out += i + 2 < bytes.length ? chars[n & 63] : "=";
  }
  return out;
}

function __start(create) {
  var challenge = create[0];
  if (create.length > 1 && typeof create[1] === "string") {
    var raw = __b64ToBytes(create[1]), text = "";
    for (var i = 0; i < raw.length; i++) text += String.fromCharCode((raw[i] + 97) & 255);
    challenge = JSON.parse(decodeURIComponent(escape(text)));
  }
  var interpreter = (challenge[1] || []).find(function (x) { return typeof x === "string"; });
  var program = challenge[4], globalName = challenge[5];
  new Function(interpreter)();
  var vm = globalThis[globalName];
  if (!vm || !vm.a) throw new Error("BotGuard VM not found");
  var fns = null;
  vm.a(program, function (snapshot, shutdown, passEvent, checkCamera) { fns = { snapshot: snapshot }; }, true, undefined, function () {}, [[], []]);
  var waited = 0;
  var poll = setInterval(function () {
    if (fns && fns.snapshot) {
      clearInterval(poll);
      fns.snapshot(function (response) { __bgResponse = response || ""; if (!response) __error = "empty snapshot"; },
        [undefined, undefined, __signals, undefined]);
    } else if (++waited > 5000) { clearInterval(poll); __error = "no snapshot function"; }
  }, 1);
}

function __integrity(it) {
  var getMinter = __signals[0];
  if (typeof getMinter !== "function") { __error = "no minter factory"; return; }
  Promise.resolve(getMinter(__b64ToBytes(it[0]))).then(function (m) { __minter = m; }, function (e) { __error = "minter: " + e; });
}

function __mint(identifier) {
  var bytes = new Uint8Array(unescape(encodeURIComponent(identifier)).split("").map(function (c) { return c.charCodeAt(0); }));
  Promise.resolve(__minter(bytes)).then(function (r) { __token = __bytesToB64Url(r); }, function (e) { __error = "mint: " + e; });
}
