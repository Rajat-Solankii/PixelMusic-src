// Dumps jsdom's DOM surface (interfaces, inheritance, members and default values) as JSON for the stand-in.
import fs from "node:fs";
import { createRequire } from "node:module";

const dir = process.argv[2];
const { JSDOM } = createRequire(dir + "/jsdomenv/package.json")("jsdom");
const w = new JSDOM("<!DOCTYPE html><html><head></head><body></body></html>", { url: "https://music.youtube.com/", pretendToBeVisual: true }).window;
const doc = w.document;

const TAGS = ["a", "abbr", "address", "area", "article", "aside", "audio", "b", "base", "bdi", "bdo", "blockquote", "body", "br", "button",
  "canvas", "caption", "cite", "code", "col", "colgroup", "data", "datalist", "dd", "del", "details", "dfn", "dialog", "div", "dl", "dt", "em",
  "embed", "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hr", "html", "i", "iframe",
  "img", "input", "ins", "kbd", "label", "legend", "li", "link", "main", "map", "mark", "menu", "meta", "meter", "nav", "noscript", "object",
  "ol", "optgroup", "option", "output", "p", "param", "picture", "pre", "progress", "q", "s", "samp", "script", "section", "select", "slot",
  "small", "source", "span", "strong", "style", "sub", "summary", "sup", "table", "tbody", "td", "template", "textarea", "tfoot", "th", "thead",
  "time", "title", "tr", "track", "u", "ul", "var", "video", "wbr", "frame", "frameset", "marquee", "font", "center"];

const interfaces = {};
function value(v) {
  if (v === null) return { t: "null" };
  if (v === undefined) return { t: "undefined" };
  if (typeof v === "string") return { t: "string", v };
  if (typeof v === "number") return { t: "number", v: Number.isFinite(v) ? v : 0 };
  if (typeof v === "boolean") return { t: "boolean", v };
  if (typeof v === "function") return { t: "function" };
  return { t: "object", tag: Object.prototype.toString.call(v).slice(8, -1) };
}
function describe(proto, sample) {
  const ctor = proto.constructor;
  const name = ctor && ctor.name;
  if (!name || interfaces[name] || proto === w.Object.prototype) return name;
  const parentProto = Object.getPrototypeOf(proto);
  const parent = parentProto && parentProto !== w.Object.prototype ? describe(parentProto, sample) : null;
  const members = {};
  for (const key of Object.getOwnPropertyNames(proto)) {
    if (key === "constructor") continue;
    const d = Object.getOwnPropertyDescriptor(proto, key);
    if (typeof d.value === "function") { members[key] = { t: "method" }; continue; }
    if (d.get) {
      let v;
      try { v = sample ? d.get.call(sample) : undefined; } catch { v = undefined; }
      members[key] = { ...value(v), setter: !!d.set };
      continue;
    }
    members[key] = value(d.value);
  }
  const constants = {};
  for (const key of Object.getOwnPropertyNames(ctor)) if (/^[A-Z_]+$/.test(key) && typeof ctor[key] === "number") constants[key] = ctor[key];
  interfaces[name] = { parent, members, constants };
  return name;
}

const tags = {};
for (const tag of TAGS) {
  const el = doc.createElement(tag);
  tags[tag] = describe(Object.getPrototypeOf(el), el);
}
describe(Object.getPrototypeOf(doc), doc);
describe(Object.getPrototypeOf(doc.createEvent("MouseEvents")), doc.createEvent("MouseEvents"));
describe(Object.getPrototypeOf(doc.createEvent("Event")), doc.createEvent("Event"));
describe(Object.getPrototypeOf(doc.createEvent("UIEvents")), doc.createEvent("UIEvents"));
describe(Object.getPrototypeOf(doc.createEvent("KeyboardEvent")), doc.createEvent("KeyboardEvent"));
describe(Object.getPrototypeOf(doc.createTextNode("x")), doc.createTextNode("x"));
describe(Object.getPrototypeOf(doc.createComment("x")), doc.createComment("x"));
describe(Object.getPrototypeOf(doc.createDocumentFragment()), doc.createDocumentFragment());
describe(Object.getPrototypeOf(doc.body.classList), doc.body.classList);
describe(Object.getPrototypeOf(doc.body.childNodes), doc.body.childNodes);
describe(Object.getPrototypeOf(doc.getElementsByTagName("x")), doc.getElementsByTagName("x"));
describe(Object.getPrototypeOf(doc.body.attributes), doc.body.attributes);
describe(Object.getPrototypeOf(doc.body.dataset), doc.body.dataset);
const events = { MouseEvents: "MouseEvent", MouseEvent: "MouseEvent", Event: "Event", Events: "Event", HTMLEvents: "Event", UIEvents: "UIEvent",
  UIEvent: "UIEvent", KeyboardEvent: "KeyboardEvent", KeyEvents: "KeyboardEvent" };
fs.writeFileSync(dir + "/dom_surface.json", JSON.stringify({ interfaces, tags, events }));
console.log(Object.keys(interfaces).length, "interfaces", Object.keys(tags).length, "tags");
process.exit(0);
