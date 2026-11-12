import init, {
  compute,
  matching,
  move_node,
  preview,
} from "./pkg/stufftracker_core";
declare const WASM_BASE64: string;
declare const WORKER_SOURCE: string;
type Point = { x: number; y: number };
type Faceplate = "panel" | "switch" | "terminals";
type Port = {
  id: string;
  label?: string;
  position?: number;
  poles?: number;
  half?: "A" | "B";
  /** Per pole: "A", "B" or "" (whole space), for linked half-space breakers. */
  halves?: string[];
  /** The supply (main breaker, main lugs, uplink), drawn above the positions. */
  main?: boolean;
  /** Breaker or circuit rating, as printed on the handle. */
  amps?: number;
  protection?: "gfci" | "afci" | "dual";
  properties?: Property[];
};
type Entity = {
  id: string;
  name: string;
  kind: string;
  parent: string | null;
  links: { provider: string; kind: string; external_id: string }[];
  faceplate?: Faceplate;
  ports?: Port[];
  properties?: Property[];
};
type PropertyValue =
  | { type: "text"; text: string }
  | { type: "link"; url: string; title?: string }
  | {
      type: "file";
      attachment: string;
      name: string;
      mime: string;
      size: number;
    };
type Property = { key: string } & PropertyValue;
type Role = "source" | "target" | "peer";
type Endpoint = { entity: string; role: Role; port?: string };
type Connection = {
  id: string;
  name: string;
  layer: string;
  endpoints: Endpoint[];
  properties?: Property[];
};
type Direction = "both" | "upstream" | "downstream";
type Op = "contains" | "equals" | "exists" | "missing";
type Condition = { key: string; op: Op; value?: string };
type Filter = { conditions: Condition[]; any?: boolean };
type Focus = {
  filter?: Filter;
  entities: string[];
  hops?: number;
  direction?: Direction;
  /** The focus's own arrangement; never the full view's. */
  positions?: Record<string, Point>;
};
type PortBox = {
  id: string;
  label: string;
  position: string;
  x: number;
  y: number;
  width: number;
  height: number;
  side: "left" | "right" | "either";
  /** A linked breaker's later, tied segment (its first is 0 / absent). */
  segment?: number;
  supply?: boolean;
};
type DocumentData = {
  entities: Entity[];
  connections: Connection[];
  layers: { id: string; name: string; color: string }[];
};
type View = {
  attachment_hints?: Record<string, Point>;
  layout_version?: number;
  positions: Record<string, Point>;
  collapsed: string[];
  hidden_layers: string[];
  faceplates?: string[];
  focus?: Focus;
  /** Spaces drawn as containers; off lays everything out flat. */
  areas?: boolean;
  flat_positions?: Record<string, Point>;
};
type Layout = {
  attachment_hints: Record<string, Point>;
  layout_version: number;
  unrouted_connections: string[];
  positions: Record<string, Point>;
  focus_positions?: Record<string, Point>;
  flat_positions?: Record<string, Point>;
  nodes: {
    id: string;
    position: Point;
    children: number;
    width: number;
    height: number;
    depth: number;
    container: boolean;
    collapsed: boolean;
    faceplate?: {
      kind: Faceplate;
      ports: PortBox[];
      empty: PortBox[];
      unassigned?: PortBox;
    };
  }[];
  bundles: {
    layer: string;
    connections: string[];
    junctions: Point[];
    label_position: Point | null;
    branches: { entity: string; role: Role; points: Point[] }[];
  }[];
};
type SavedView = { id: string; name: string; view: View };
const $ = <T extends HTMLElement>(id: string) =>
  document.getElementById(id) as T;
const esc = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
const icon = (kind: string) =>
  ({ space: "▧", device: "◈", equipment: "▤", service: "◇" })[kind] || "◈";
let doc: DocumentData,
  view: View = { positions: {}, collapsed: [], hidden_layers: [] },
  revision = 0;
let offline = false,
  writable = false,
  selected = "kitchen",
  dirty = false,
  savedViews: SavedView[] = [],
  current: Layout;
// Things picked with shift-click or a shift-drag lasso, for "show only these".
let picked = new Set<string>();
// The list (the home screen) or a diagram. The list never lays anything out;
// a diagram is drawn only when asked for: of the checked things, a saved
// view, or everything.
let mode: "list" | "diagram" = "list";
// Checked in the list, kept as the query changes, for "Diagram these".
const checked = new Set<string>();
let listQuery = "";
const listOpen = new Set<string>();
let camera = { x: 0, y: 0, scale: 1 },
  first = true;
const svg = $("graph") as unknown as SVGSVGElement;
function status(text: string, error = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", error);
}
async function api(path: string, options?: RequestInit) {
  const r = await fetch(`api/${path}`, options);
  if (!r.ok) throw new Error(await r.text());
  return r.status === 204 ? null : r.json();
}
function run(fn: () => unknown | Promise<unknown>) {
  return () => {
    try {
      Promise.resolve(fn()).catch((e) => status(String(e.message || e), true));
    } catch (e) {
      status(String(e), true);
    }
  };
}
function computeLayout() {
  applyLayout(JSON.parse(compute(JSON.stringify({ document: doc, view }))));
}
function applyLayout(layout: Layout) {
  current = layout;
  view.attachment_hints = current.attachment_hints;
  view.positions = current.positions;
  view.layout_version = current.layout_version;
  // A focus keeps the arrangement it was laid out in (and dragged into).
  if (view.focus && layout.focus_positions)
    view.focus.positions = layout.focus_positions;
  // Flat (areas off) keeps its own arrangement beside the nested one.
  else if (layout.flat_positions) view.flat_positions = layout.flat_positions;
}
// A trace ("what feeds this", "show only this", a filter) opens as a dialog
// over what you were looking at; before is that, for Back.
let traceOpen = false;
let before: { mode: "list" | "diagram"; focus?: Focus } | null = null;
function openTrace(focus: Focus) {
  if (!traceOpen)
    before = {
      mode,
      ...(view.focus ? { focus: structuredClone(view.focus) } : {}),
    };
  setFocus(focus, true);
}
// What the trace shows: its things, not the spaces drawn only around them.
function shownThings() {
  const named = new Set(view.focus?.entities || []);
  return current.nodes
    .filter((n) => !n.container || n.collapsed || named.has(n.id))
    .map((n) => n.id);
}
function closeTrace(how: "keep" | "add" | "back") {
  if (how === "back") {
    // Nothing to go back to (a diagram of the checks): the list.
    const b = before ?? { mode: "list" as const };
    traceOpen = false;
    before = null;
    if (b.mode === "list") {
      if (b.focus) view.focus = b.focus;
      else delete view.focus;
      mode = "list";
      render();
    } else setFocus(b.focus || null);
    return;
  }
  const shown = shownThings();
  if (how === "keep") checked.clear();
  shown.forEach((id) => checked.add(id));
  setFocus({ entities: [...checked], hops: 0, direction: "both" });
  status(
    how === "keep"
      ? `Kept ${shown.length} things, checked`
      : `Added ${shown.length} things: ${checked.size} checked`,
  );
}
function setFocus(focus: Focus | null, trace = false) {
  traceOpen = trace;
  if (!trace) before = null;
  mode = "diagram";
  if (focus) view.focus = focus;
  else delete view.focus;
  picked.clear();
  render();
  fit();
}
const nameOf = (id: string) =>
  doc.entities.find((e) => e.id === id)?.name || id;
// --- Live status: Home Assistant states and Outline titles -----------------
// Fetched on their own, after drawing, so status never delays layout or
// rendering; a slow or unwired bridge just means no status yet.
type EntityStatus = {
  state: string;
  name: string;
  changed: number;
  attributes: Record<string, string | number | boolean>;
};
type LiveStatus = {
  connected: boolean;
  error?: string;
  updated?: number;
  entities?: Record<string, EntityStatus>;
  areas?: Record<string, { name: string; entities: string[] }>;
  devices?: Record<string, DeviceStatus>;
};
// A Home Assistant device: its entities, those per port number, and the
// disabled ones (per-port traffic, say) that could be turned on in HA.
type DeviceStatus = {
  entities: { id: string; category: string }[];
  ports: Record<string, string[]>;
  disabled: string[];
};
type LiveDocuments = {
  connected: boolean;
  documents?: Record<string, { title?: string; url?: string; error?: string }>;
};
let liveStatus: LiveStatus | null = null;
let liveDocuments: LiveDocuments | null = null;
const isHa = (p: string) =>
  p === "home_assistant" || p === "homeassistant" || p === "ha";
type Tone = "on" | "off" | "alert" | "value" | "muted";
// A state as people say it: "on 78%", "21.4 °C", "open", "unavailable".
function formatState(
  id: string,
  s: EntityStatus,
): { text: string; tone: Tone } {
  const domain = id.split(".")[0];
  const a = s.attributes || {};
  const dc = String(a.device_class || "");
  if (s.state === "unavailable" || s.state === "unknown")
    return { text: s.state, tone: "muted" };
  const onOff = (on: string, off: string, alert = false) =>
    s.state === "on"
      ? { text: on, tone: (alert ? "alert" : "on") as Tone }
      : { text: off, tone: "off" as Tone };
  switch (domain) {
    case "light":
      return s.state === "on" && typeof a.brightness === "number"
        ? { text: `on ${Math.round((a.brightness / 255) * 100)}%`, tone: "on" }
        : onOff("on", "off");
    case "switch":
    case "fan":
    case "input_boolean":
    case "automation":
    case "siren":
      return onOff("on", "off");
    case "binary_sensor":
      if (["door", "window", "garage_door", "opening"].includes(dc))
        return onOff("open", "closed", true);
      if (["motion", "occupancy", "presence"].includes(dc))
        return onOff("motion", "clear");
      if (
        ["moisture", "smoke", "gas", "problem", "safety", "tamper"].includes(dc)
      )
        return onOff(dc === "moisture" ? "wet" : "alert", "ok", true);
      if (dc === "connectivity") return onOff("connected", "disconnected");
      if (dc === "battery") return onOff("low", "ok", true);
      return onOff("on", "off");
    case "sensor": {
      if (s.state === "connected") return { text: "online", tone: "value" };
      if (s.state === "disconnected") return { text: "offline", tone: "alert" };
      const n = Number(s.state);
      const unit = a.unit_of_measurement ? ` ${a.unit_of_measurement}` : "";
      const v =
        Number.isFinite(n) && s.state.trim() !== ""
          ? Number.isInteger(n)
            ? String(n)
            : n.toFixed(1)
          : s.state;
      return { text: `${v}${unit}`, tone: "value" };
    }
    case "climate": {
      const t = a.current_temperature;
      return {
        text: `${t !== undefined ? `${t}°` : s.state}${a.hvac_action ? ` ${a.hvac_action}` : ` ${s.state}`}`,
        tone:
          a.hvac_action === "heating" || a.hvac_action === "cooling"
            ? "on"
            : "value",
      };
    }
    case "device_tracker":
      // A router-seen tracker (network gear, anything wired) is about being
      // reachable, not about being "home".
      if (a.source_type === "router" || a.tracking_type === "connection")
        return s.state === "home"
          ? { text: "online", tone: "value" }
          : { text: "offline", tone: "alert" };
      return s.state === "home"
        ? { text: "home", tone: "on" }
        : { text: s.state === "not_home" ? "away" : s.state, tone: "off" };
    case "lock":
      return { text: s.state, tone: s.state === "unlocked" ? "alert" : "off" };
    case "cover":
      return { text: s.state, tone: s.state === "open" ? "on" : "off" };
    default:
      return {
        text: s.state.length > 16 ? s.state.slice(0, 15) + "…" : s.state,
        tone: "value",
      };
  }
}
// A room linked to an area: only what's worth a glance. Its temperature
// from a room thermometer or thermostat (not a printer bed, a target or a
// switch's internal sensor), lights that are on (not device LEDs), and
// anything open. Nothing worth saying means no pill.
const ROOM_TEMP =
  /thermostat|thermometer|air_temp|room_temp|_temperature_temperature$/;
const NOT_ROOM_TEMP =
  /target|internal|nozzle|bed_|chamber|cpu|battery|water|probe/;
function areaSummary(areaId: string): { text: string; tone: Tone } | null {
  const area = liveStatus?.areas?.[areaId];
  const states = liveStatus?.entities;
  if (!area || !states) return null;
  const members = area.entities
    .map((id) => [id, states[id]] as const)
    .filter(([, s]) => s);
  const parts: string[] = [];
  const temp = members.find(
    ([id, s]) =>
      id.startsWith("sensor.") &&
      s.attributes.device_class === "temperature" &&
      ROOM_TEMP.test(id) &&
      !NOT_ROOM_TEMP.test(id) &&
      Number.isFinite(Number(s.state)),
  );
  if (temp) parts.push(formatState(temp[0], temp[1]).text);
  const on = members.filter(
    ([id, s]) =>
      id.startsWith("light.") && !/_led$/.test(id) && s.state === "on",
  ).length;
  if (on) parts.push(`${on} light${on === 1 ? "" : "s"} on`);
  const open = members.filter(([id, s]) => {
    if (!id.startsWith("binary_sensor.")) return false;
    return (
      ["door", "window", "garage_door", "opening"].includes(
        String(s.attributes.device_class),
      ) && s.state === "on"
    );
  }).length;
  if (open) parts.push(`${open} open`);
  if (!parts.length) return null;
  return {
    text: parts.join(" · "),
    tone: open ? "alert" : on ? "on" : "value",
  };
}
// A thing's Home Assistant devices, from its device links.
function devicesOf(e: Entity): DeviceStatus[] {
  return e.links
    .filter((l) => isHa(l.provider) && liveStatus?.devices?.[l.external_id])
    .map((l) => liveStatus!.devices![l.external_id]);
}
const isDeviceLink = (l: { kind: string; external_id: string }) =>
  l.kind === "device" || /^[0-9a-f]{32}$/i.test(l.external_id);
// Is this state about being reachable (online/offline)?
const isPresence = (id: string, s: EntityStatus) =>
  id.startsWith("device_tracker.") ||
  (id.startsWith("binary_sensor.") &&
    s.attributes.device_class === "connectivity") ||
  (id.startsWith("sensor.") &&
    (s.state === "connected" || s.state === "disconnected"));
// Readings worth a glance on a card: power, temperature, clients…
const READING_CLASSES = ["power", "temperature", "humidity", "battery"];
const isReading = (id: string, s: EntityStatus) =>
  id.startsWith("sensor.") &&
  Number.isFinite(Number(s.state)) &&
  s.state.trim() !== "" &&
  (READING_CLASSES.includes(String(s.attributes.device_class)) ||
    /_clients$/.test(id));
// What a thing's card shows. Every linked entity (and each linked device's
// own entities, ports aside) is a candidate: a problem wins, then whether
// it's online, then its first plain state; one reading rides along.
function cardStatus(e: Entity): { text: string; tone: Tone } | null {
  if (!liveStatus?.connected) return null;
  const states = liveStatus.entities || {};
  const ids: string[] = [];
  const ha = e.links.filter((l) => isHa(l.provider));
  for (const l of ha) {
    if (l.kind === "area") continue;
    if (isDeviceLink(l)) {
      const d = liveStatus.devices?.[l.external_id];
      const perPort = new Set(Object.values(d?.ports || {}).flat());
      for (const x of d?.entities || [])
        if (!perPort.has(x.id) && x.category !== "config") ids.push(x.id);
    } else ids.push(l.external_id);
  }
  const known = [...new Set(ids)]
    .filter((id) => states[id])
    .map((id) => ({ id, s: states[id], f: formatState(id, states[id]) }));
  const plain = known.filter(
    (k) => k.f.tone !== "muted" && !isReading(k.id, k.s),
  );
  const primary =
    plain.find((k) => k.f.tone === "alert" && !k.id.startsWith("update.")) ||
    plain.find((k) => isPresence(k.id, k.s)) ||
    plain.find(
      (k) =>
        !k.id.startsWith("button.") &&
        !k.id.startsWith("update.") &&
        !k.id.startsWith("number.") &&
        !k.id.startsWith("select.") &&
        !k.id.startsWith("image."),
    );
  const reading = known.find((k) => isReading(k.id, k.s));
  if (primary || reading) {
    // Offline readings are stale: say only that it's offline.
    const parts = [
      primary?.f.text,
      primary?.f.tone === "alert" ? "" : reading?.f.text,
    ];
    return {
      text: parts.filter(Boolean).join(" · "),
      tone: (primary || reading)!.f.tone,
    };
  }
  if (known.length) return known[0].f;
  for (const l of ha.filter((l) => l.kind === "area")) {
    const a = areaSummary(l.external_id);
    if (a) return a;
  }
  return null;
}
// A port's live state, from its thing's devices' entities for that port
// number: PoE and its draw, the port switched off, traffic when enabled.
function portLive(
  e: Entity,
  p?: Port,
  withLink = true,
): { ids: string[]; text: string; tone: Tone } | null {
  if (!p?.position || !liveStatus?.entities) return null;
  const ids = devicesOf(e).flatMap((d) => d.ports[String(p.position)] || []);
  if (!ids.length) return null;
  const states = liveStatus.entities;
  const known = ids.filter((id) => states[id]);
  const find = (re: RegExp) => known.find((id) => re.test(id));
  const enable = find(/^switch\..*_port_\d+$/);
  if (enable && states[enable].state === "off")
    return { ids, text: "off", tone: "off" };
  const parts: string[] = [];
  let tone: Tone = "muted";
  // Link speed (Mbit/s) says whether anything is plugged in: 0 is down.
  const speedId = find(/_link_speed$/);
  const speed = speedId ? Number(states[speedId].state) : NaN;
  if (Number.isFinite(speed) && withLink) {
    if (speed <= 0) return { ids, text: "down", tone: "off" };
    parts.push(speed >= 1000 ? `${speed / 1000}G` : `${speed}M`);
    tone = "value";
  }
  const poe = find(/^switch\..*_poe$/);
  const watts = known.find(
    (id) =>
      id.startsWith("sensor.") &&
      states[id].attributes.device_class === "power",
  );
  const w = watts ? Number(states[watts].state) : NaN;
  if (Number.isFinite(w) && w > 0) {
    parts.push(`${w < 10 ? w.toFixed(1) : Math.round(w)} W`);
    tone = "on";
  } else if (poe && states[poe].state === "off") {
    parts.push("PoE off");
    tone = "off";
  }
  const rate = (re: RegExp) => {
    const id = find(re);
    const n = id ? Number(states[id].state) : NaN;
    return Number.isFinite(n) ? n : null;
  };
  const rx = rate(/_rx$/),
    tx = rate(/_tx$/);
  if (rx !== null || tx !== null) {
    const mb = (n: number) => (n < 10 ? n.toFixed(1) : String(Math.round(n)));
    parts.push(`↓${mb(rx || 0)} ↑${mb(tx || 0)}`);
    if (tone === "muted") tone = (rx || 0) + (tx || 0) > 0 ? "on" : "value";
  }
  if (!parts.length) return { ids, text: "", tone };
  return { ids, text: parts.join(" "), tone };
}
// A switch port's link light, like the switch's own app: gigabit, fast
// ethernet, nothing plugged in, or switched off; and whether it has PoE
// (it has PoE entities, even turned-off ones) and is powering something.
type LinkLight = "gbe" | "fe" | "down" | "off" | "unknown";
function portLink(
  e: Entity,
  p?: Port,
): { link: LinkLight; poe: "" | "poe" | "powered"; title: string } {
  const none = { link: "unknown" as LinkLight, poe: "" as const, title: "" };
  if (!p?.position || !liveStatus?.entities) return none;
  const devices = devicesOf(e);
  const ids = devices.flatMap((d) => d.ports[String(p.position)] || []);
  const here = new RegExp(`_port_${p.position}_`);
  const disabled = devices
    .flatMap((d) => d.disabled)
    .filter((id) => here.test(id));
  const states = liveStatus.entities;
  const find = (re: RegExp) => ids.find((id) => states[id] && re.test(id));
  let link: LinkLight = "unknown";
  const enable = find(/^switch\..*_port_\d+$/);
  const speedId = find(/_link_speed$/);
  const speed = speedId ? Number(states[speedId].state) : NaN;
  if (enable && states[enable].state === "off") link = "off";
  else if (Number.isFinite(speed))
    link = speed >= 1000 ? "gbe" : speed > 0 ? "fe" : "down";
  const watts = ids.find(
    (id) => states[id]?.attributes.device_class === "power",
  );
  const w = watts ? Number(states[watts].state) : NaN;
  const hasPoe = [...ids, ...disabled].some((id) => /_poe(_power)?$/.test(id));
  const poe = Number.isFinite(w) && w > 0 ? "powered" : hasPoe ? "poe" : "";
  const title = [
    {
      gbe: `link ${speed >= 1000 ? speed / 1000 + " Gbit/s" : ""}`,
      fe: `link ${speed} Mbit/s`,
      down: "nothing plugged in",
      off: "port turned off",
      unknown: "",
    }[link],
    poe === "powered" ? `PoE ${w} W` : poe ? "PoE" : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return { link, poe, title };
}
// A status pill, right-aligned at (right, top) in the node's coordinates.
function pillSvg(e: Entity, right: number, top: number, max: number) {
  const st = cardStatus(e);
  if (!st) return "";
  const text = st.text.length > max ? st.text.slice(0, max - 1) + "…" : st.text;
  const w = Math.max(24, text.length * 5.6 + 14);
  return `<g class="ha-pill ${st.tone}" transform="translate(${right - w},${top})"><rect width="${w}" height="15" rx="7.5"/><text x="${w / 2}" y="8">${esc(text)}</text></g>`;
}
const ago = (ms: number) => {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  return s < 90
    ? `${Math.round(s)}s ago`
    : s < 5400
      ? `${Math.round(s / 60)} min ago`
      : s < 129600
        ? `${Math.round(s / 3600)} h ago`
        : `${Math.round(s / 86400)} d ago`;
};
// One Home Assistant entity: name, id, when it changed, a few attributes.
function haRow(id: string) {
  const st = liveStatus?.entities?.[id];
  if (!st)
    return `<div class="ha-row"><div><b>${esc(id)}</b><small>${liveStatus?.connected ? "not reported" : ""}</small></div></div>`;
  const f = formatState(id, st);
  const attrs = Object.entries(st.attributes)
    .filter(([k]) => !["unit_of_measurement", "device_class"].includes(k))
    .slice(0, 6)
    .map(([k, v]) => `${k.replaceAll("_", " ")}: ${v}`)
    .join(" · ");
  return `<div class="ha-row"><div><b>${esc(st.name || id)}</b><small>${esc(id)}${st.changed ? ` · changed ${ago(st.changed)}` : ""}</small>${attrs ? `<small class="ha-attrs">${esc(attrs)}</small>` : ""}</div><span class="ha-state ${f.tone}">${esc(f.text)}</span></div>`;
}
// A linked device: its main entities, then the housekeeping ones folded
// away; per-port entities show on each port instead.
function deviceHtml(id: string) {
  const d = liveStatus?.devices?.[id];
  if (!d)
    return `<div class="ha-row"><div><b>Device</b><small>${esc(id)}${liveStatus?.connected ? " · not found" : ""}</small></div></div>`;
  const perPort = new Set(Object.values(d.ports).flat());
  const rest = d.entities.filter((x) => !perPort.has(x.id));
  const main = rest.filter((x) => !x.category);
  const more = rest.filter((x) => x.category);
  const ports = Object.keys(d.ports).length;
  const note = [
    `${d.entities.length} entities`,
    ports ? `${ports} ports — select a port for its state` : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return `<div class="ha-device"><small class="muted">Device · ${esc(note)}</small>${main.map((x) => haRow(x.id)).join("")}${more.length ? `<details><summary>${more.length} more (diagnostic, config)</summary>${more.map((x) => haRow(x.id)).join("")}</details>` : ""}</div>`;
}
// A port's Home Assistant entities, in the port inspector.
function portLiveHtml(e: Entity, p: Port) {
  if (!e.links?.some((l) => isHa(l.provider) && isDeviceLink(l)))
    return '<div class="port-live"></div>';
  const live = portLive(e, p);
  const disabled = devicesOf(e)
    .flatMap((d) => d.disabled)
    .filter((id) => new RegExp(`_port_${p.position}_`).test(id));
  const body = !liveStatus
    ? '<p class="muted">Loading status…</p>'
    : !live
      ? '<p class="muted">No Home Assistant entities for this port.</p>'
      : live.ids.map(haRow).join("");
  const hint = disabled.length
    ? `<p class="muted">Turned off in Home Assistant: ${disabled.map((id) => esc(id.split(".")[1])).join(", ")}</p>`
    : "";
  return `<div class="port-live ha-links"><h3>Home Assistant</h3>${body}${hint}</div>`;
}
// The inspector's live links: Home Assistant states and Outline documents.
function liveLinksHtml(e: Entity) {
  const links = e.links || [];
  if (!links.length) return "";
  const ha = links.filter((l) => isHa(l.provider));
  const docs = links.filter((l) => l.provider === "outline");
  const other = links.filter(
    (l) => !isHa(l.provider) && l.provider !== "outline",
  );
  let html = "";
  if (ha.length) {
    const rows = ha.flatMap((l) => {
      if (l.kind === "area") {
        const area = liveStatus?.areas?.[l.external_id];
        const sum = areaSummary(l.external_id);
        return [
          `<div class="ha-row"><div><b>${esc(area?.name || l.external_id)}</b><small>area</small></div>${sum ? `<span class="ha-state ${sum.tone}">${esc(sum.text)}</span>` : ""}</div>`,
        ];
      }
      if (isDeviceLink(l)) return [deviceHtml(l.external_id)];
      return [haRow(l.external_id)];
    });
    const note = !liveStatus
      ? '<p class="muted">Loading status…</p>'
      : !liveStatus.connected
        ? '<p class="muted">Home Assistant isn\'t connected to StuffTracker.</p>'
        : liveStatus.error
          ? `<p class="muted error">${esc(liveStatus.error)}</p>`
          : "";
    html += `<div class="ha-links"><h3>Home Assistant</h3>${note}${rows.join("")}</div>`;
  }
  if (docs.length)
    html += `<div class="doc-links"><h3>Documents</h3>${docs
      .map((l) => {
        const d = liveDocuments?.documents?.[l.external_id];
        return d?.url
          ? `<a href="${esc(d.url)}" target="_blank" rel="noopener noreferrer">📄 ${esc(d.title || "Outline document")}</a>`
          : `<span class="muted">📄 ${liveDocuments && !liveDocuments.connected ? "Outline document (Outline isn't connected)" : liveDocuments ? "Outline document" : "Loading…"}</span>`;
      })
      .join("")}</div>`;
  html += other
    .map(
      (l) =>
        `<div class="external-link"><b>${esc(l.provider.replaceAll("_", " ").toUpperCase())} · ${esc(l.kind)}</b>${esc(l.external_id)}</div>`,
    )
    .join("");
  return `<div class="live-links">${html}</div>`;
}
// Repaint only what status touches: pills (scene markup, no layout) and the
// live sections of the inspector and the phone sheet.
function paintLive() {
  if (mode === "list") renderListRows();
  else if (!dragBase) renderScene();
  for (const root of [$("inspector"), $("sheet")]) {
    const box = root.querySelector(".live-links");
    const e = doc.entities.find(
      (x) => x.id === (root.id === "sheet" ? sheetId : selected),
    );
    if (box && e) box.outerHTML = liveLinksHtml(e);
    const portBox = root.querySelector(".port-live");
    const sel =
      root.id === "sheet"
        ? sheetTarget?.kind === "port"
          ? { entity: sheetTarget.id, port: sheetTarget.port }
          : null
        : selectedPort;
    const pe = sel && doc.entities.find((x) => x.id === sel.entity);
    const pp = pe && pe.ports?.find((x) => x.id === sel!.port);
    if (portBox && pe && pp) portBox.outerHTML = portLiveHtml(pe, pp);
  }
}
let liveTimer = 0;
async function refreshLive(documents = false) {
  if (offline) return;
  const want = doc.entities.some((e) => e.links?.some((l) => isHa(l.provider)));
  const wantDocs = doc.entities.some((e) =>
    e.links?.some((l) => l.provider === "outline"),
  );
  try {
    if (want && !document.hidden)
      liveStatus = await (await fetch("api/live/status")).json();
    if (documents && wantDocs)
      liveDocuments = await (await fetch("api/live/documents")).json();
    paintLive();
  } catch {
    /* status is best-effort; try again next round */
  }
}
function startLive() {
  void refreshLive(true);
  clearInterval(liveTimer);
  let round = 0;
  liveTimer = window.setInterval(() => {
    round++;
    void refreshLive(round % 120 === 0);
  }, 15000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) void refreshLive();
  });
}
// --- Filters: show only the things whose details match -------------------
const OPS: Record<Op, string> = {
  contains: "contains",
  equals: "is",
  exists: "has",
  missing: "doesn't have",
};
const FIELD_NAMES: Record<string, string> = {
  "@any": "Any field",
  "@name": "Name",
  "@kind": "Kind",
};
const fieldName = (key: string) => FIELD_NAMES[key] || key;
const conditionReady = (c: Condition) =>
  c.op === "exists" || c.op === "missing" || !!(c.value || "").trim();
function describeFilter(f: Filter) {
  return f.conditions
    .map((c) =>
      c.op === "exists" || c.op === "missing"
        ? `${OPS[c.op]} ${fieldName(c.key)}`
        : `${fieldName(c.key)} ${OPS[c.op]} "${c.value || ""}"`,
    )
    .join(f.any ? " or " : " and ");
}
const matchingIds = (filter: Filter): string[] =>
  JSON.parse(matching(JSON.stringify({ document: doc, filter })));
function showMatching(filter: Filter) {
  // In the list, "show everything with this detail" is a query there.
  if (mode === "list") {
    sheetTarget = null;
    sheetId = null;
    renderSheet();
    setListQuery(filterToQuery(filter));
    return;
  }
  openTrace({ entities: [], hops: 0, direction: "both", filter });
}
// The filter being edited in the panel, until shown.
let filterDraft: Filter | null = null;
function openFilterPanel() {
  filterDraft = structuredClone(
    view.focus?.filter || {
      conditions: [{ key: "@any", op: "contains", value: "" }],
    },
  );
  renderFilterPanel();
}
function filterCount(f: Filter) {
  const live = f.conditions.filter(conditionReady);
  const n = live.length ? matchingIds({ ...f, conditions: live }).length : 0;
  return {
    live: live.length,
    n,
    text: live.length ? `${n} match${n === 1 ? "" : "es"}` : "",
  };
}
function renderFilterPanel() {
  const panel = $("filter-panel");
  panel.hidden = !filterDraft;
  if (!filterDraft) return;
  const f = filterDraft;
  // Every detail key in the inventory, most used first.
  const counts = new Map<string, number>();
  const everyDetail = [
    ...doc.entities.flatMap((e) => [
      ...(e.properties || []),
      ...(e.ports || []).flatMap((p) => p.properties || []),
    ]),
    ...doc.connections.flatMap((c) => c.properties || []),
  ];
  for (const p of everyDetail) counts.set(p.key, (counts.get(p.key) || 0) + 1);
  const keys = [...counts]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([k]) => k);
  const count = filterCount(f);
  panel.innerHTML = `<div class="filter-head"><b>Show things where</b><span class="seg" role="group" aria-label="Match"><button data-any="false" class="${f.any ? "" : "on"}">all</button><button data-any="true" class="${f.any ? "on" : ""}">any</button></span><span class="muted">match</span><button class="filter-close" aria-label="Close filter">✕</button></div>${f.conditions
    .map((c, i) => {
      const special = c.key.startsWith("@");
      const options = [
        ...Object.keys(FIELD_NAMES),
        ...keys,
        ...(special || keys.includes(c.key) ? [] : [c.key]),
      ];
      return `<div class="filter-row" data-cond="${i}"><select data-field="key" aria-label="Field">${options.map((k) => `<option value="${esc(k)}" ${k === c.key ? "selected" : ""}>${esc(fieldName(k))}</option>`).join("")}</select><select data-field="op" aria-label="Test">${(
        Object.keys(OPS) as Op[]
      )
        .filter((op) => !special || op === "contains" || op === "equals")
        .map(
          (op) =>
            `<option value="${op}" ${op === c.op ? "selected" : ""}>${OPS[op]}</option>`,
        )
        .join(
          "",
        )}</select>${c.op === "exists" || c.op === "missing" ? "<span></span>" : `<input data-field="value" value="${esc(c.value || "")}" placeholder="value" aria-label="Value">`}<button class="remove-cond" aria-label="Remove condition">✕</button></div>`;
    })
    .join(
      "",
    )}<div class="filter-foot"><button id="add-cond">＋ Condition</button><span class="filter-count">${count.text}</span><button id="apply-filter" class="primary" ${count.n ? "" : "disabled"}>Show matches</button></div>`;
  panel.querySelector<HTMLButtonElement>(".filter-close")!.onclick = () => {
    filterDraft = null;
    renderFilterPanel();
  };
  panel.querySelectorAll<HTMLButtonElement>("[data-any]").forEach(
    (b) =>
      (b.onclick = () => {
        f.any = b.dataset.any === "true";
        renderFilterPanel();
      }),
  );
  panel.querySelectorAll<HTMLElement>(".filter-row").forEach((row) => {
    const i = Number(row.dataset.cond);
    const c = f.conditions[i];
    row.querySelector<HTMLSelectElement>('[data-field="key"]')!.onchange = (
      ev,
    ) => {
      c.key = (ev.target as HTMLSelectElement).value;
      if (c.key.startsWith("@") && (c.op === "exists" || c.op === "missing"))
        c.op = "contains";
      renderFilterPanel();
    };
    row.querySelector<HTMLSelectElement>('[data-field="op"]')!.onchange = (
      ev,
    ) => {
      c.op = (ev.target as HTMLSelectElement).value as Op;
      renderFilterPanel();
    };
    const input = row.querySelector<HTMLInputElement>('[data-field="value"]');
    if (input) {
      // Update the count without redrawing (which would lose the caret).
      input.oninput = () => {
        c.value = input.value;
        const now = filterCount(f);
        panel.querySelector(".filter-count")!.textContent = now.text;
        panel.querySelector<HTMLButtonElement>("#apply-filter")!.disabled =
          !now.n;
      };
      input.onkeydown = (ev) => {
        if (ev.key === "Enter") $("apply-filter").click();
      };
    }
    row.querySelector<HTMLButtonElement>(".remove-cond")!.onclick = () => {
      f.conditions.splice(i, 1);
      if (!f.conditions.length)
        f.conditions.push({ key: "@any", op: "contains", value: "" });
      renderFilterPanel();
    };
  });
  $("add-cond").onclick = () => {
    f.conditions.push(
      keys.length
        ? { key: keys[0], op: "exists" }
        : { key: "@any", op: "contains", value: "" },
    );
    renderFilterPanel();
  };
  $("apply-filter").onclick = () => {
    const conditions = f.conditions
      .filter(conditionReady)
      .map((c) =>
        c.op === "exists" || c.op === "missing"
          ? { key: c.key, op: c.op }
          : { ...c, value: c.value!.trim() },
      );
    if (!conditions.length) return;
    filterDraft = null;
    renderFilterPanel();
    showMatching({ conditions, ...(f.any ? { any: true } : {}) });
  };
}
function renderBars() {
  if (mode === "list" || !current) return;
  // The bar is the trace's dialog: a diagram of the checks or a saved view
  // opens without one (the toolbar's List goes back).
  const f = traceOpen ? view.focus : undefined;
  const bar = $("focus-bar");
  bar.hidden = !f;
  if (!f) bar.innerHTML = "";
  if (f) {
    const names = f.entities.map(nameOf);
    const what = [
      ...(f.filter ? [describeFilter(f.filter)] : []),
      names.length > 2
        ? `${names.slice(0, 2).join(", ")} +${names.length - 2}`
        : names.join(", "),
    ]
      .filter(Boolean)
      .join(" + ");
    const dir = f.direction || "both";
    const lead =
      f.filter && !f.hops
        ? "Matching"
        : dir === "upstream"
          ? "What feeds"
          : dir === "downstream"
            ? "What is fed by"
            : "Around";
    // Every focus has a way back (to the list, if nothing else); Keep and
    // Add only when it shows something other than what's checked.
    const shown = shownThings();
    const same =
      shown.length === checked.size && shown.every((id) => checked.has(id));
    const toList = (before?.mode ?? "list") === "list";
    bar.innerHTML = `<span class="focus-label">${lead} <b>${esc(what)}</b> · ${current.nodes.filter((n) => !n.container || n.collapsed).length} shown</span><span class="seg" role="group" aria-label="Direction">${(["both", "upstream", "downstream"] as const).map((d) => `<button data-dir="${d}" class="${d === dir ? "on" : ""}">${{ both: "↔ Both", upstream: "↑ Feeds", downstream: "↓ Fed" }[d]}</button>`).join("")}</span><span class="seg" role="group" aria-label="Hops">${(["0", "1", "2", "3", "all"] as const).map((h) => `<button data-hops="${h}" class="${String(f.hops ?? "all") === h ? "on" : ""}" title="${h === "0" ? "Only these things" : h === "all" ? "Everything connected" : `${h} connection${h === "1" ? "" : "s"} away`}">${h === "0" ? "Only" : h === "all" ? "All" : h}</button>`).join("")}</span><span class="trace-exits" role="group" aria-label="Close">${
      same
        ? ""
        : `<button id="trace-keep" class="primary" title="Check just these and show only them">✓ Keep these</button><button id="trace-add" title="Add these to what's checked, and show them all">＋ Add<span class="long"> to checked</span>${checked.size ? ` (${checked.size})` : ""}</button>`
    }<button id="trace-back" title="${toList ? "Back to the list" : "Back to what you were looking at"}">${toList ? "☰ List" : "↩ Back"}</button></span>`;
    bar
      .querySelectorAll<HTMLButtonElement>("[data-dir]")
      .forEach(
        (b) =>
          (b.onclick = () =>
            setFocus(
              { ...view.focus!, direction: b.dataset.dir as Direction },
              traceOpen,
            )),
      );
    bar.querySelectorAll<HTMLButtonElement>("[data-hops]").forEach(
      (b) =>
        (b.onclick = () =>
          setFocus(
            {
              ...view.focus!,
              hops:
                b.dataset.hops === "all" ? undefined : Number(b.dataset.hops),
            },
            traceOpen,
          )),
    );
    if (!same) {
      $("trace-keep").onclick = () => closeTrace("keep");
      $("trace-add").onclick = () => closeTrace("add");
    }
    $("trace-back").onclick = () => closeTrace("back");
  }
  const pick = $("pick-bar");
  pick.hidden = !picked.size;
  if (picked.size) {
    pick.innerHTML = `<span>${picked.size} selected</span><button id="focus-picked" class="primary">Show only these</button><button id="clear-picked">Clear</button>`;
  } else if (selectMode) {
    pick.hidden = false;
    pick.innerHTML = `<span>Tap things to pick them, or drag to lasso</span><button id="clear-picked">Done</button>`;
    $("clear-picked").onclick = () => setSelectMode(false);
  }
  if (picked.size) {
    $("focus-picked").onclick = () =>
      openTrace({ entities: [...picked], hops: 0, direction: "both" });
    $("clear-picked").onclick = () => {
      picked.clear();
      renderScene();
      renderBars();
    };
    if (selectMode)
      $("focus-picked").addEventListener("click", () => setSelectMode(false));
  }
}
function render() {
  const listing = mode === "list";
  $("app").classList.toggle("list-mode", listing);
  $("list").hidden = !listing;
  document.querySelector(".canvas-toolbar .eyebrow")!.textContent = listing
    ? "INVENTORY"
    : "SPATIAL WORKSPACE";
  document.querySelector(".canvas-toolbar h1")!.textContent = listing
    ? "Your stuff"
    : "House connections";
  if (listing) {
    renderList();
    renderLayers();
    renderInspector();
    renderSheet();
    $<HTMLButtonElement>("save-document").disabled = !writable || !dirty;
    $("counts").textContent =
      `${doc.entities.length} things · ${doc.connections.length} connections`;
    $("counts").classList.remove("error");
    return;
  }
  $<HTMLInputElement>("areas").checked = !!view.areas;
  // Re-arranging is the flat layout's: nested spaces keep theirs.
  $("rearrange").hidden = !!view.areas;
  layoutGeneration++;
  delete svg.dataset.layoutPending;
  computeLayout();
  renderScene();
  renderTree();
  renderLayers();
  renderInspector();
  renderBars();
  renderSheet();

  $<HTMLButtonElement>("save-document").disabled = !writable || !dirty;
  if (first) {
    first = false;
    fit();
  }
}
function renderScene() {
  if (mode === "list" || !current) return;
  let content = "";
  for (const l of doc.layers) {
    content += `<defs><marker id="arrow-${esc(l.id)}" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="${esc(l.color)}"/></marker></defs>`;
  }
  // Paint the containing spaces first, then routes, then device cards. Their
  // world rectangles are supplied by Rust; the TypeScript shell does no layout.
  for (const n of current.nodes
    .filter((n) => n.container && !n.collapsed)
    .sort((a, b) => a.depth - b.depth)) {
    const e = doc.entities.find((e) => e.id === n.id)!;
    const fills = ["#edf2e7", "#f5f8f0", "#fcfdf9"];
    const headers = ["#e0ead9", "#e9f0e1", "#f0f5e9"];
    content += `<g class="node space ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="space-body" width="${n.width}" height="${n.height}" rx="12" fill="${fills[Math.min(n.depth, 2)]}"/><g class="space-header"><path d="M12 0 H${n.width - 12} Q${n.width} 0 ${n.width} 12 V52 H0 V12 Q0 0 12 0 Z" fill="${headers[Math.min(n.depth, 2)]}"/><path class="space-divider" d="M0 52 H${n.width}"/><text class="glyph" x="15" y="35">${icon(e.kind)}</text><text class="kind" x="43" y="18">${esc(e.kind.toUpperCase())}</text><text class="name" x="43" y="36">${esc(e.name)}</text><text class="space-count" x="${n.width - 50}" y="32" text-anchor="end">${n.children} inside</text>${pillSvg(e, n.width - 44, 5, 30)}${n.children ? `<g class="fold" data-fold="${esc(n.id)}"><rect x="${n.width - 34}" y="16" width="20" height="20" rx="5"/><text x="${n.width - 29}" y="30">−</text></g>` : ""}</g></g>`;
  }
  for (const b of current.bundles) {
    const layer = doc.layers.find((l) => l.id === b.layer)!;
    for (const branch of b.branches) {
      if (!branch.points.length) continue;
      const path = branch.points
        .map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`)
        .join(" ");
      // A wide invisible stroke makes the wire easy to click.
      content += `<path class="route-hit" d="${path}" data-connections="${esc(b.connections.join(" "))}"/><path class="route ${selectedConnection && b.connections.includes(selectedConnection) ? "selected-wire" : ""}" data-connections="${esc(b.connections.join(" "))}" stroke="${esc(layer.color)}" d="${path}" ${branch.role === "target" ? `marker-start="url(#arrow-${esc(layer.id)})"` : ""}><title>${esc(b.connections.map((id) => doc.connections.find((c) => c.id === id)!.name).join(", "))}</title></path>`;
    }
    for (const point of b.junctions) {
      content += `<circle class="junction" data-connection="${esc(b.connections[0])}" cx="${point.x}" cy="${point.y}" r="4" fill="${esc(layer.color)}"><title>${esc(b.connections.map((id) => doc.connections.find((c) => c.id === id)!.name).join(", "))} · branch point</title></circle>`;
    }
    if (b.connections.length > 1 && b.label_position)
      content += `<text class="bundle-count" x="${b.label_position.x + 7}" y="${b.label_position.y - 7}">${b.connections.length} links</text>`;
  }
  for (const n of current.nodes.filter((n) => !n.container || n.collapsed)) {
    const e = doc.entities.find((e) => e.id === n.id)!;
    if (n.faceplate) {
      content += faceplateSvg(n, e);
      continue;
    }
    content += `<g class="node ${n.collapsed ? "collapsed-space" : ""} ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="card" width="${n.width}" height="${n.height}" rx="9"/><text class="glyph" x="15" y="42">${icon(e.kind)}</text><text class="kind" x="45" y="23">${esc(e.kind.toUpperCase())}</text>${pillSvg(e, n.width - 8, 6, 18)}<text class="name" x="45" y="44">${esc(e.name.length > 19 ? e.name.slice(0, 18) + "…" : e.name)}</text>${n.children ? `<g class="fold" data-fold="${esc(e.id)}"><rect x="173" y="10" width="19" height="19" rx="4" fill="#e2edd8"/><text x="178" y="24">+</text></g><text class="kind" x="16" y="62">${n.children} hidden inside</text>` : ""}</g>`;
  }
  document.getElementById("scene")!.innerHTML = content;
  applyCamera();
  const missing = current.unrouted_connections.length;
  const routed = current.bundles.filter((b) =>
    b.branches.every((branch) => branch.points.length),
  ).length;
  $("counts").textContent =
    `${current.nodes.length} of ${doc.entities.length} things · ${routed} visible connections${missing ? ` · ${missing} need more routing space` : ""}`;
  $("counts").classList.toggle("error", missing > 0);
}
// A faceplate: the header, then each port (or panel breaker) as a slot in
// its physical position, with what it feeds; Rust supplies the geometry.
function faceplateSvg(n: Layout["nodes"][number], e: Entity) {
  const f = n.faceplate!;
  const fed = (port: string) =>
    doc.connections
      .filter((c) =>
        c.endpoints.some((ep) => ep.entity === e.id && ep.port === port),
      )
      .flatMap((c) =>
        c.endpoints
          .filter((ep) => ep.entity !== e.id)
          .map((ep) => doc.entities.find((x) => x.id === ep.entity)!.name),
      );
  const portOf = (id: string) => e.ports?.find((p) => p.id === id);
  const panel = f.kind === "panel";
  const HANDLE = 15;
  // On a panel, each breaker's handle faces the bus: the inner edge of its
  // column (the right end of a main row), with its rating printed sideways.
  const handleX = (b: PortBox) =>
    b.side === "right" ? b.x + 2 : b.x + b.width - HANDLE - 2;
  const handle = (b: PortBox, p?: Port) => {
    if (!panel || !b.id) return "";
    const x = handleX(b),
      h = Math.max(4, b.height - 4);
    const amps = p?.amps && !b.segment ? String(p.amps) : "";
    const cx = x + HANDLE / 2,
      cy = b.y + 2 + h / 2;
    // Sideways like a breaker handle where there is room; a half-height
    // tandem handle is too short for that, so its rating sits upright.
    const upright = h < 12;
    const size = upright
      ? Math.min(7, (HANDLE - 2) / Math.max(1, amps.length * 0.6))
      : Math.max(6, Math.min(9, (h - 3) / Math.max(1, amps.length * 0.62)));
    return `<rect class="handle" x="${x}" y="${b.y + 2}" width="${HANDLE}" height="${h}" rx="2"/>${amps ? `<text class="amps" style="font-size:${size}px" transform="translate(${cx},${cy})${upright ? "" : " rotate(-90)"}">${amps}</text>` : ""}`;
  };
  const PROTECTION = { gfci: "GF", afci: "AF", dual: "DF" } as const;
  const badge = (b: PortBox, p?: Port) => {
    if (!p?.protection || b.segment) return "";
    const w = 15,
      h = Math.min(10, b.height - 2);
    const x = panel
      ? b.side === "right"
        ? b.x + HANDLE + 5
        : b.x + b.width - HANDLE - w - 5
      : b.x + b.width - w - 4;
    return `<g class="badge ${p.protection}"><rect x="${x}" y="${b.y + (b.height - h) / 2}" width="${w}" height="${h}" rx="2"/><text x="${x + w / 2}" y="${b.y + b.height / 2}">${PROTECTION[p.protection]}</text></g>`;
  };
  // Space numbers sit at the outer edge of each column (right-aligned on
  // the right), and a breaker covering several spaces lists them one per
  // row, beside each space it fills.
  const ROW = 24;
  const positions = (b: PortBox, x: number, end: boolean) => {
    const parts = (b.position || "·").split("/");
    const anchor = end ? ' text-anchor="end"' : "";
    const fit =
      parts.length < 2
        ? null
        : b.height >= parts.length * ROW - 6
          ? { pitch: ROW, first: 13, cls: "" }
          : b.height >= parts.length * 10
            ? {
                pitch: 10,
                first: b.height / 2 - ((parts.length - 1) * 10) / 2,
                cls: " tiny mid",
              }
            : null;
    if (!fit) {
      const small = b.height < 14;
      return `<text class="slot-pos${small ? " tiny mid" : ""}" x="${x}" y="${b.y + (small ? b.height / 2 : 13)}"${anchor}>${esc(b.position || "·")}</text>`;
    }
    return parts
      .map(
        (t, k) =>
          `<text class="slot-pos${fit.cls}" x="${x}" y="${b.y + fit.first + k * fit.pitch}"${anchor}>${esc(t)}</text>`,
      )
      .join("");
  };
  // On a switch, each port number sits in its link light, with a bolt
  // for PoE (bright when it powers something).
  const lightSq = (b: PortBox, p?: Port) => {
    const l = p
      ? portLink(e, p)
      : { link: "unknown" as LinkLight, poe: "", title: "" };
    // Mirrored on the right column, like a panel: the light at the outer
    // edge, its number in the outer corner, the bolt toward the middle.
    const mirror = b.side === "right";
    const h = b.height - 4,
      x = mirror ? b.x + b.width - 29 : b.x + 3,
      y = b.y + 2;
    const bolt = b.supply
      ? `<path class="uplink" transform="translate(${mirror ? x + 3 : x + 17},${y + 3})" d="M0 5.5 L3.2 1.5 L6.4 5.5"/>`
      : l.poe
        ? `<path class="bolt" transform="translate(${mirror ? x + 3 : x + 17},${y + 2})" d="M4.2 0 L0 5.4 H2.8 L1.8 9.5 L6.2 3.6 H3.4 Z"/>`
        : "";
    return `<g class="link-light ${l.link} ${l.poe}"><rect x="${x}" y="${y}" width="26" height="${h}" rx="3"/><text x="${mirror ? x + 22 : x + 4}" y="${y + h - 4}"${mirror ? ' text-anchor="end"' : ""}>${esc(b.position || "·")}</text>${bolt}</g>`;
  };
  // A port's live state (PoE draw, off, traffic) at its outer edge.
  const liveTag = (b: PortBox, p?: Port) => {
    const l = panel || b.segment ? null : portLive(e, p, f.kind !== "switch");
    if (!l?.text || b.height < 12) return { svg: "", width: 0 };
    const w = l.text.length * 4.6 + 8;
    // Inside the light on a mirrored switch column, else at the outer edge.
    const x =
      f.kind === "switch" && b.side === "right"
        ? b.x + b.width - 32 - w
        : b.x + b.width - w - (p?.protection ? 22 : 3);
    return {
      svg: `<g class="port-tag ${l.tone}"><rect x="${x}" y="${b.y + 2}" width="${w}" height="${Math.min(12, b.height - 4)}" rx="3"/><text x="${x + w / 2}" y="${b.y + 2 + Math.min(12, b.height - 4) / 2}">${esc(l.text)}</text></g>`,
      width: w + 3,
    };
  };
  const slot = (b: PortBox, cls: string, title: string) => {
    const p = b.id ? portOf(b.id) : undefined;
    const tag = liveTag(b, p);
    const named =
      [b.label, ...(b.id ? fed(b.id) : [])].filter(Boolean)[0] || "";
    // A supply in its own place is marked (on a switch, in its light).
    const text = b.supply && f.kind !== "switch" ? `▲ ${named}` : named;
    const right = panel && b.side === "right";
    // Past the handle (and badge) on the right column; past the number on
    // the left.
    const handleW = panel && b.id ? HANDLE + 4 : 0;
    const badgeW = p?.protection ? 19 : 0;
    const labelX = right
      ? b.x + handleW + badgeW + 5
      : f.kind === "switch" && b.side === "right"
        ? b.x + 8
        : b.x + 38;
    const posX = right ? b.x + b.width - 5 : b.x + 5;
    const max = Math.max(
      4,
      Math.floor((b.width - 44 - handleW - badgeW - tag.width) / 6.2),
    );
    const small = b.height < 14;
    const number =
      f.kind === "switch" && b.height >= 14 && b !== f.unassigned
        ? lightSq(b, p)
        : positions(b, posX, right);
    const link = f.kind === "switch" && p ? portLink(e, p).title : "";
    return `<g class="slot ${cls}" ${b.id ? `data-port="${esc(b.id)}"` : ""}><rect x="${b.x}" y="${b.y}" width="${b.width}" height="${b.height}" rx="3"/>${number}<text class="slot-label ${small ? "tiny mid" : ""}" x="${labelX}" y="${b.y + (small ? b.height / 2 : 13)}">${esc(text.length > max ? text.slice(0, max - 1) + "…" : text)}</text>${handle(b, p)}${badge(b, p)}${tag.svg}<title>${esc(title)}${link ? `\n${esc(link)}` : ""}${tag.svg ? `\nHome Assistant: ${esc(portLive(e, p)!.text)}` : ""}</title></g>`;
  };
  const rating = (p?: Port) =>
    [
      p?.amps ? `${p.amps} A` : "",
      p?.protection ? PROTECTION_NAME[p.protection] : "",
    ]
      .filter(Boolean)
      .join(" ");
  let body = "",
    ties = "";
  for (const b of f.empty)
    body += slot(b, "empty", `Space ${b.position}: empty`);
  for (const b of f.ports) {
    if (b.segment) {
      // A linked breaker's later space: tie its handle to the first one's,
      // like the handle tie on the breaker itself.
      const first = f.ports.find((p) => p.id === b.id && !p.segment)!;
      // Beside the handles, so it never hides a rating it crosses.
      const x = panel
        ? b.side === "right"
          ? handleX(b) + HANDLE + 3
          : handleX(b) - 3
        : b.x + b.width - 7;
      const right = panel && b.side === "right";
      const labelX = right ? b.x + HANDLE + 9 : b.x + 38;
      body += `<g class="slot used tied"><rect x="${b.x}" y="${b.y}" width="${b.width}" height="${b.height}" rx="3"/>${positions(b, right ? b.x + b.width - 5 : b.x + 5, right)}<text class="slot-label tiny mid" x="${labelX}" y="${b.y + b.height / 2}">↑ ${esc(b.id)}</text>${handle(b, portOf(b.id))}</g>`;
      // Drawn last, over any breaker the tie crosses (a quad's inner pair).
      ties += `<path class="tie" d="M${x},${first.y + first.height / 2} V${b.y + b.height / 2}"><title>${esc(b.id)}: one linked breaker</title></path>`;
      continue;
    }
    const names = fed(b.id);
    const r = rating(portOf(b.id));
    const extra = (portOf(b.id)?.properties || [])
      .map(
        (d) =>
          `\n${d.key}: ${d.type === "text" ? d.text : d.type === "link" ? d.title || d.url : d.name}`,
      )
      .join("");
    body += slot(
      b,
      names.length ? "used" : "spare",
      `${b.position ? b.position + " · " : ""}${b.id}${r ? " · " + r : ""}${b.label ? " · " + b.label : ""}${names.length ? "\n→ " + names.join(", ") : "\nNothing connected"}${extra}`,
    );
  }
  if (f.unassigned)
    body += slot(
      f.unassigned,
      "unassigned",
      "Connections not yet assigned to a port",
    );
  body += ties;
  const kindLabel = {
    panel: "PANEL",
    switch: "SWITCH",
    terminals: "TERMINALS",
  }[f.kind];
  return `<g class="node faceplate ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="card" width="${n.width}" height="${n.height}" rx="9"/><text class="glyph" x="15" y="34">${icon(e.kind)}</text><text class="kind" x="45" y="20">${esc(e.kind.toUpperCase())} · ${kindLabel}</text><text class="name" x="45" y="39">${esc(e.name.length > 34 ? e.name.slice(0, 33) + "…" : e.name)}</text>${pillSvg(e, n.width - 10, 8, 26)}${body}</g>`;
}
// What the inventory search matches: the name and every detail.
const searchText = (e: Entity) =>
  [
    e.name,
    ...[
      ...(e.properties || []),
      ...(e.ports || []).flatMap((p) => p.properties || []),
    ].flatMap((p) => [
      p.key,
      p.type === "text"
        ? p.text
        : p.type === "link"
          ? `${p.url} ${p.title || ""}`
          : p.name,
    ]),
  ]
    .join("\n")
    .toLowerCase();
function renderTree() {
  const query = $<HTMLInputElement>("search").value.toLowerCase();
  let html = "";
  function walk(parent: string | null, depth: number) {
    for (const e of doc.entities.filter((e) => e.parent === parent)) {
      const children = doc.entities.some((n) => n.parent === e.id);
      if (!query || searchText(e).includes(query))
        html += `<div class="tree-row ${selected === e.id ? "selected" : ""}" data-select="${esc(e.id)}" style="padding-left:${depth * 13 + 3}px"><button data-fold="${esc(e.id)}" ${children ? "" : "disabled"} aria-label="Toggle ${esc(e.name)}">${children ? (view.collapsed.includes(e.id) ? "▸" : "▾") : "·"}</button><span class="kind-icon">${icon(e.kind)}</span><span>${esc(e.name)}</span></div>`;
      if (!view.collapsed.includes(e.id) || query) walk(e.id, depth + 1);
    }
  }
  walk(null, 0);
  $("tree").innerHTML = html;
}
// --- The list: every thing as an expandable tree, with a typed filter ------
// A query is terms, all of which must hold: a word anywhere; key:value (a
// detail whose key starts with key, e.g. ip:192.168); key=value (exactly);
// has:key; name:, kind:, in:space, status:offline, is:checked; and -term
// for "not". Quote anything with spaces: "IP address":10.0 or "back door".
type Term = { neg: boolean; key: string | null; exact: boolean; value: string };
const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
function parseQuery(q: string): Term[] {
  const out: Term[] = [];
  for (const raw of q.match(/(?:[^\s"]+|"[^"]*"?)+/g) || []) {
    let t = raw;
    const neg = t.length > 1 && t.startsWith("-");
    if (neg) t = t.slice(1);
    // The first separator outside quotes splits key from value.
    let quoted = false,
      at = -1;
    for (let i = 0; i < t.length; i++) {
      if (t[i] === '"') quoted = !quoted;
      else if (!quoted && (t[i] === ":" || t[i] === "=")) {
        at = i;
        break;
      }
    }
    const unq = (x: string) => x.replaceAll('"', "").trim();
    if (at > 0)
      out.push({
        neg,
        key: unq(t.slice(0, at)),
        exact: t[at] === "=",
        value: unq(t.slice(at + 1)),
      });
    else if (unq(t)) out.push({ neg, key: null, exact: false, value: unq(t) });
  }
  return out;
}
const quoteTerm = (s: string) =>
  /[\s:="]/.test(s) ? `"${s.replaceAll('"', "")}"` : s;
// A filter (from a detail's "same value" or key button) as a typed query.
function filterToQuery(f: Filter) {
  return f.conditions
    .map((c) => {
      const key =
        c.key === "@name"
          ? "name"
          : c.key === "@kind"
            ? "kind"
            : quoteTerm(c.key);
      const v = quoteTerm(c.value || "");
      if (c.op === "exists") return `has:${key}`;
      if (c.op === "missing") return `-has:${key}`;
      if (c.key === "@any") return v;
      return `${key}${c.op === "equals" ? "=" : ":"}${v}`;
    })
    .join(" ");
}
const propertyText = (p: Property) =>
  p.type === "text"
    ? p.text
    : p.type === "link"
      ? `${p.url} ${p.title || ""}`
      : p.name;
// Everything a query can look at on a thing: its details, its ports and
// their details, its connections and theirs, and its external links.
function fieldsOf(e: Entity) {
  const prop = (p: Property) => ({ key: p.key, text: propertyText(p) });
  return [
    ...(e.properties || []).map(prop),
    ...(e.ports || []).flatMap((p) => [
      { key: "port", text: `${p.id} ${p.label || ""}` },
      ...(p.properties || []).map(prop),
    ]),
    ...doc.connections
      .filter((c) => c.endpoints.some((ep) => ep.entity === e.id))
      .flatMap((c) => [
        { key: "connection", text: c.name },
        ...(c.properties || []).map(prop),
      ]),
    ...(e.links || []).map((l) => ({
      key: l.provider,
      text: `${l.kind} ${l.external_id}`,
    })),
  ];
}
function ancestorsOf(e: Entity) {
  const out: Entity[] = [];
  let p = e.parent;
  while (p) {
    const a = doc.entities.find((x) => x.id === p);
    if (!a || out.includes(a)) break;
    out.push(a);
    p = a.parent;
  }
  return out;
}
function termHolds(
  e: Entity,
  t: Term,
  fields: { key: string; text: string }[],
) {
  const v = t.value.toLowerCase();
  const has = (s: string) =>
    t.exact ? s.trim().toLowerCase() === v : s.toLowerCase().includes(v);
  const k = t.key ? norm(t.key) : "";
  let ok: boolean;
  if (!t.key)
    ok =
      has(e.name) ||
      has(e.kind) ||
      fields.some((f) => has(f.key) || has(f.text));
  else if (k === "name") ok = has(e.name);
  else if (k === "kind") ok = has(e.kind);
  else if (k === "in") ok = ancestorsOf(e).some((a) => has(a.name));
  else if (k === "is") ok = v === "checked" ? checked.has(e.id) : false;
  else if (k === "status") {
    const st = cardStatus(e);
    ok = !!st && (!v || has(st.text) || st.tone === v);
  } else if (k === "has")
    ok = !v || fields.some((f) => norm(f.key).startsWith(norm(t.value)));
  else {
    const fs = fields.filter((f) => norm(f.key).startsWith(k));
    ok = v ? fs.some((f) => has(f.text)) : fs.length > 0;
  }
  return t.neg ? !ok : ok;
}
function setListQuery(q: string) {
  listQuery = q;
  const input = document.querySelector<HTMLInputElement>("#list-query");
  if (input && input.value !== q) input.value = q;
  renderListRows();
}
// The list's frame (query box) is drawn once; rows redraw as things change.
function renderList() {
  const root = $("list");
  if (!root.querySelector("#list-query")) {
    root.innerHTML = `<div class="list-head"><label class="list-search"><span>⌕</span><input id="list-query" type="search" autocomplete="off" spellcheck="false" placeholder="Filter: words, ip:192.168, kind:switch, in:barn, has:manual, status:offline" aria-label="Filter the list"></label><div id="list-actions" class="list-actions"></div></div><div id="list-rows" class="list-rows" role="tree"></div>`;
    const input = root.querySelector<HTMLInputElement>("#list-query")!;
    input.value = listQuery;
    input.oninput = () => {
      listQuery = input.value;
      renderListRows();
    };
    root.querySelector<HTMLElement>("#list-rows")!.onclick = onListClick;
    root.querySelector<HTMLElement>("#list-actions")!.onclick = onListAction;
    for (const e of doc.entities) if (!e.parent) listOpen.add(e.id);
  }
  renderListRows();
}
let listMatches: string[] = [];
function renderListRows() {
  const rowsBox = document.querySelector<HTMLElement>("#list-rows");
  if (!rowsBox) return;
  const terms = parseQuery(listQuery);
  const active = terms.length > 0;
  const kids = new Map<string | null, Entity[]>();
  for (const e of doc.entities) {
    const k = e.parent || null;
    if (!kids.has(k)) kids.set(k, []);
    kids.get(k)!.push(e);
  }
  const match = new Set<string>();
  if (active)
    for (const e of doc.entities) {
      const fields = fieldsOf(e);
      if (terms.every((t) => termHolds(e, t, fields))) match.add(e.id);
    }
  listMatches = [...match];
  const shown = new Set(match);
  for (const id of match)
    for (const a of ancestorsOf(doc.entities.find((x) => x.id === id)!))
      shown.add(a.id);
  // Key terms say which details matter: show those first in each row.
  const keyed = terms
    .filter(
      (t) =>
        t.key &&
        !["name", "kind", "in", "is", "status", "has"].includes(norm(t.key)),
    )
    .map((t) => norm(t.key!));
  const checkedInside = (id: string): number =>
    (kids.get(id) || []).reduce(
      (n, c) => n + (checked.has(c.id) ? 1 : 0) + checkedInside(c.id),
      0,
    );
  const countInside = (id: string): number =>
    (kids.get(id) || []).reduce((n, c) => n + 1 + countInside(c.id), 0);
  let html = "";
  const walk = (parent: string | null, depth: number) => {
    for (const e of kids.get(parent) || []) {
      if (active && !shown.has(e.id)) continue;
      const children = (kids.get(e.id) || []).length;
      const open = active || listOpen.has(e.id);
      const facts = (e.properties || [])
        .filter((p) => p.type === "text" || p.type === "link")
        .map((p) => ({
          p,
          rank: keyed.some((k) => norm(p.key).startsWith(k)) ? 0 : 1,
        }))
        .sort((a, b) => a.rank - b.rank)
        .slice(0, 2)
        .map(
          ({ p }) =>
            `<span><i>${esc(p.key)}</i> ${esc(propertyText(p))}</span>`,
        )
        .join("");
      const st = cardStatus(e);
      const inside = children && !open ? checkedInside(e.id) : 0;
      html += `<div class="list-row ${active && !match.has(e.id) ? "context" : ""} ${selected === e.id ? "selected" : ""} ${checked.has(e.id) ? "checked" : ""}" data-row="${esc(e.id)}" role="treeitem" style="--depth:${depth}"${children ? ` aria-expanded="${open}"` : ""}><button class="twisty" data-twisty="${esc(e.id)}" ${children && !active ? "" : "disabled"} aria-label="${open ? "Collapse" : "Expand"} ${esc(e.name)}">${children ? (open ? "▾" : "▸") : ""}</button><input type="checkbox" data-check="${esc(e.id)}" ${checked.has(e.id) ? "checked" : ""} aria-label="Check ${esc(e.name)}"><span class="kind-icon">${icon(e.kind)}</span><span class="list-name"><b>${esc(e.name)}</b><small>${esc(e.kind)}${children ? ` · ${countInside(e.id)} inside` : ""}${inside ? ` · <em>${inside} checked inside</em>` : ""}</small></span><span class="list-facts">${facts}</span><span class="list-status">${st ? `<span class="ha-state ${st.tone}">${esc(st.text)}</span>` : ""}</span></div>`;
      if (children && open) walk(e.id, depth + 1);
    }
  };
  walk(null, 0);
  rowsBox.innerHTML =
    html ||
    `<p class="muted list-empty">Nothing matches <b>${esc(listQuery)}</b>.</p>`;
  renderListActions(active, match);
}
function renderListActions(active: boolean, match: Set<string>) {
  const box = document.querySelector<HTMLElement>("#list-actions");
  if (!box) return;
  const unchecked = [...match].filter((id) => !checked.has(id)).length;
  const checkedHere = [...match].filter((id) => checked.has(id)).length;
  const n = checked.size;
  box.innerHTML = `<span class="list-count">${active ? `<b>${match.size}</b> of ${doc.entities.length} match` : `${doc.entities.length} things`}${n ? ` · <b>${n}</b> checked` : ""}</span>${active && unchecked ? `<button data-list="check">Check ${unchecked === match.size ? "all " : ""}${unchecked} match${unchecked === 1 ? "" : "es"}</button>` : ""}${active && checkedHere ? `<button data-list="uncheck">Uncheck ${checkedHere}</button>` : ""}${n ? `<button data-list="clear">Clear checks</button>` : ""}<span class="list-spacer"></span>${writable ? `<button data-list="add" title="Add a thing">＋ Add</button>` : ""}<button data-list="everything" title="Diagram everything (slow with a big inventory)">Everything</button><button data-list="diagram" class="primary" ${n ? "" : "disabled"}>Diagram ${n || ""} →</button>`;
}
function onListAction(ev: MouseEvent) {
  const b = (ev.target as HTMLElement).closest<HTMLButtonElement>(
    "[data-list]",
  );
  if (!b) return;
  const what = b.dataset.list;
  if (what === "check") listMatches.forEach((id) => checked.add(id));
  else if (what === "uncheck") listMatches.forEach((id) => checked.delete(id));
  else if (what === "clear") checked.clear();
  else if (what === "add") return $("add").click();
  else if (what === "everything") return setFocus(null);
  else if (what === "diagram")
    return setFocus({ entities: [...checked], hops: 0, direction: "both" });
  renderListRows();
}
function onListClick(ev: MouseEvent) {
  const target = ev.target as HTMLElement;
  const row = target.closest<HTMLElement>("[data-row]");
  if (!row) return;
  const id = row.dataset.row!;
  if (target.matches("[data-check]")) {
    const box = target as HTMLInputElement;
    if (box.checked) checked.add(id);
    else checked.delete(id);
    row.classList.toggle("checked", box.checked);
    const terms = parseQuery(listQuery);
    renderListActions(terms.length > 0, new Set(listMatches));
    return;
  }
  if (target.closest("[data-twisty]")) {
    if (listOpen.has(id)) listOpen.delete(id);
    else listOpen.add(id);
    renderListRows();
    return;
  }
  selectEntity(id);
  document
    .querySelectorAll("#list-rows .list-row.selected")
    .forEach((r) => r.classList.remove("selected"));
  row.classList.add("selected");
  renderInspector();
  if (narrow()) openSheet({ kind: "entity", id });
}
function renderLayers() {
  $("layers").innerHTML = doc.layers
    .map(
      (l) =>
        `<label class="layer"><span class="swatch" style="background:${esc(l.color)}"></span><span>${esc(l.name)}</span><small>${doc.connections.filter((c) => c.layer === l.id).length}</small><input type="checkbox" data-layer="${esc(l.id)}" aria-label="${esc(l.name)} layer" ${view.hidden_layers.includes(l.id) ? "" : "checked"}></label>`,
    )
    .join("");
}
const PROTECTION_NAME = {
  gfci: "GFCI",
  afci: "AFCI",
  dual: "AFCI/GFCI",
} as const;
// Supplies with no position (they're drawn on top) first, then by position.
const topMain = (p: Port) => !!p.main && !p.position;
const portOrder = (a: Port, b: Port) =>
  Number(topMain(b)) - Number(topMain(a)) ||
  (a.position ?? 1e9) - (b.position ?? 1e9) ||
  (a.halves?.[0] || a.half || "").localeCompare(
    b.halves?.[0] || b.half || "",
  ) ||
  a.id.localeCompare(b.id, undefined, { numeric: true });
function spaces(e: Entity, p: Port) {
  if (topMain(p)) return "Main";
  if (!p.position) return "";
  const step = e.faceplate === "panel" ? 2 : 1;
  const list = Array.from(
    { length: Math.max(1, p.poles || 1) },
    (_, k) => p.position! + k * step,
  );
  return p.halves?.length
    ? list.map((n, k) => `${n}${p.halves![k] || ""}`).join("/")
    : list.join("/") + (p.half || "");
}
// What each port of `e` connects to: its schedule, plus the connections not
// yet assigned to a port.
function schedule(e: Entity) {
  const linked = (c: Connection) =>
    c.endpoints
      .filter((ep) => ep.entity !== e.id)
      .map((ep) => doc.entities.find((x) => x.id === ep.entity)!.name);
  const mine = doc.connections.filter((c) =>
    c.endpoints.some((ep) => ep.entity === e.id),
  );
  const at = (c: Connection) =>
    c.endpoints.find((ep) => ep.entity === e.id)!.port;
  return {
    rows: [...(e.ports || [])].sort(portOrder).map((p) => ({
      port: p,
      position: spaces(e, p),
      connections: mine
        .filter((c) => at(c) === p.id)
        .map((c) => ({ c, to: linked(c) })),
    })),
    unassigned: mine.filter((c) => !at(c)).map((c) => ({ c, to: linked(c) })),
  };
}
const ratingText = (p: Port) =>
  [
    p.amps ? `${p.amps} A` : "",
    p.protection ? PROTECTION_NAME[p.protection] : "",
  ]
    .filter(Boolean)
    .join(" · ");
const ratingHtml = (p: Port) =>
  ratingText(p)
    ? `<span class="rating ${p.protection || ""}">${esc(ratingText(p))}</span>`
    : "";
function scheduleHtml(e: Entity) {
  if (!e.ports?.length) return "";
  const { rows, unassigned } = schedule(e);
  const layer = (c: Connection) => doc.layers.find((l) => l.id === c.layer)!;
  const cell = (list: { c: Connection; to: string[] }[]) =>
    list.length
      ? list
          .map(
            ({ c, to }) =>
              `<div><span class="swatch" style="background:${esc(layer(c).color)}"></span>${esc(to.join(", "))}<small>${esc(c.name)}</small></div>`,
          )
          .join("")
      : '<span class="muted">spare</span>';
  return `<div class="schedule"><div class="schedule-head"><h3>${e.faceplate === "panel" ? "Panel schedule" : "Ports"}</h3><button type="button" class="print-schedule">Print</button></div><table><thead><tr><th>${e.faceplate === "panel" ? "Space" : "#"}</th><th>Port</th><th>Connected to</th></tr></thead><tbody>${rows
    .map(
      (r) =>
        `<tr><td>${esc(r.position)}</td><td><button class="link-button port-id" data-select-port="${portRef(e.id, r.port.id)}"><b>${esc(r.port.id)}</b></button>${ratingHtml(r.port)}${r.port.label ? `<small>${esc(r.port.label)}</small>` : ""}</td><td>${cell(r.connections)}</td></tr>`,
    )
    .join(
      "",
    )}${unassigned.length ? `<tr class="unassigned"><td>?</td><td>Not assigned</td><td>${cell(unassigned)}</td></tr>` : ""}</tbody></table></div>`;
}
// The selected thing's connections; with ports, each can be assigned one.
function connectionsHtml(e: Entity, assign: boolean) {
  const mine = doc.connections.filter((c) =>
    c.endpoints.some((ep) => ep.entity === e.id),
  );
  if (!mine.length) return "";
  const ports = [...(e.ports || [])].sort(portOrder);
  return `<div class="conn-list"><h3>Connections</h3>${mine
    .map((c) => {
      const layer = doc.layers.find((l) => l.id === c.layer)!;
      const ep = c.endpoints.find((ep) => ep.entity === e.id)!;
      const others = c.endpoints
        .filter((x) => x.entity !== e.id)
        .map((x) => {
          const other = doc.entities.find((n) => n.id === x.entity)!;
          return `${esc(other.name)}${x.port ? ` <small>@${esc(x.port)}</small>` : ""}`;
        })
        .join(", ");
      const arrow =
        ep.role === "source" ? "→" : ep.role === "target" ? "←" : "↔";
      const select =
        assign && ports.length
          ? `<select data-assign="${esc(c.id)}" aria-label="Port for ${esc(c.name)}"><option value="">port…</option>${ports.map((p) => `<option value="${esc(p.id)}" ${ep.port === p.id ? "selected" : ""}>${esc(p.id)}${p.label ? " · " + esc(p.label) : ""}</option>`).join("")}</select>`
          : ep.port
            ? `<small>@${esc(ep.port)}</small>`
            : "";
      return `<div class="conn"><span class="swatch" style="background:${esc(layer.color)}"></span><div><button class="link-button" data-select-connection="${esc(c.id)}">${arrow} ${others}</button><small>${esc(c.name)}</small></div>${select}</div>`;
    })
    .join("")}</div>`;
}
function portEditorHtml(e: Entity) {
  const draft = draftPorts?.entity === e.id ? draftPorts : null;
  const ports: (Port & { orig?: string })[] = draft
    ? draft.ports
    : (e.ports || []).map((p) => ({ ...p, orig: p.id }));
  const faceplate = draft ? draft.faceplate : e.faceplate;
  return `<details class="port-editor" ${portEditorOpen ? "open" : ""}><summary>Physical layout &amp; ports</summary><form id="ports-form"><label class="field">Layout<select name="faceplate"><option value="">None</option>${(["panel", "switch", "terminals"] as const).map((k) => `<option value="${k}" ${faceplate === k ? "selected" : ""}>${{ panel: "Breaker panel", switch: "Network switch", terminals: "Terminal strip" }[k]}</option>`).join("")}</select></label><div class="port-rows"><div class="port-head"><span>ID</span><span title="Starting space or port number, or main">Pos</span><span title="Spaces covered (2 for a 240 V breaker)">Poles</span><span title="Tandem half: A or B, or one per space (B/A)">½</span><span></span></div>${ports
    .map(
      (p, i) =>
        `<div class="port-row" data-row="${i}" ${p.orig ? `data-orig="${esc(p.orig)}"` : ""}><div class="l1"><input name="id" value="${esc(p.id)}" required maxlength="40" aria-label="Port ID"><input name="position" inputmode="numeric" pattern="[0-9]{1,3}|[Mm][Aa][Ii][Nn]" placeholder="#" title="Starting space or port number, or main for the supply" aria-label="Position" value="${topMain(p) ? "main" : (p.position ?? "")}"><select name="poles" aria-label="Poles">${[1, 2, 3].map((n) => `<option ${(p.poles || 1) === n ? "selected" : ""}>${n}</option>`).join("")}</select><input name="half" pattern="[ABab]|([ABab-]/)+[ABab-]" title="A or B for a tandem half; one per space for a linked breaker, e.g. B/A (- for a whole space)" aria-label="Half" value="${esc(p.halves?.length ? p.halves.map((h) => h || "-").join("/") : p.half || "")}"><button type="button" class="remove-port" aria-label="Remove port ${esc(p.id)}">✕</button></div><div class="l2"><input name="amps" inputmode="numeric" pattern="[0-9]{1,4}" placeholder="A" title="Rating in amps" aria-label="Amps" value="${p.amps ?? ""}"><select name="protection" aria-label="Protection" title="Built-in protection"><option value="">—</option>${(["gfci", "afci", "dual"] as const).map((k) => `<option value="${k}" ${p.protection === k ? "selected" : ""}>${PROTECTION_NAME[k]}</option>`).join("")}</select><input name="label" value="${esc(p.label || "")}" maxlength="200" placeholder="Label" aria-label="Label"><label class="supply-check" title="The supply: an uplink, or a breaker that feeds the panel"><input type="checkbox" name="supply" ${p.main ? "checked" : ""}> Supply</label></div></div>`,
    )
    .join(
      "",
    )}</div><div class="button-row"><button type="button" id="add-port">＋ Port</button><button type="button" id="number-ports" title="Add numbered ports up to a count">＋ Numbered…</button><button>Apply ports</button></div></form></details>`;
}
let portEditorOpen = false;
// Besides the selected thing, a wire (connection) or one of a thing's ports
// can be selected for inspection.
let selectedConnection: string | null = null;
let selectedPort: { entity: string; port: string } | null = null;
function selectEntity(id: string) {
  selected = id;
  selectedConnection = null;
  selectedPort = null;
}
// Buttons that switch what is inspected: a thing, a connection, a port.
function wireSelectors(root: HTMLElement, sheet = false) {
  root.querySelectorAll<HTMLElement>("[data-select-entity]").forEach(
    (b) =>
      (b.onclick = () => {
        selectEntity(b.dataset.selectEntity!);
        if (sheet) openSheet({ kind: "entity", id: selected });
        render();
      }),
  );
  root.querySelectorAll<HTMLElement>("[data-select-connection]").forEach(
    (b) =>
      (b.onclick = (ev) => {
        ev.preventDefault();
        selectedConnection = b.dataset.selectConnection!;
        selectedPort = null;
        if (sheet) openSheet({ kind: "connection", id: selectedConnection });
        renderScene();
        renderInspector();
      }),
  );
  root.querySelectorAll<HTMLElement>("[data-select-port]").forEach(
    (b) =>
      (b.onclick = (ev) => {
        ev.preventDefault();
        const [entity, ...rest] = b.dataset.selectPort!.split(":");
        selected = entity;
        selectedConnection = null;
        selectedPort = { entity, port: rest.join(":") };
        if (sheet)
          openSheet({ kind: "port", id: entity, port: selectedPort.port });
        renderInspector();
      }),
  );
}
const portRef = (e: string, port: string) => `${esc(e)}:${esc(port)}`;
const roleArrow = (r: Role) =>
  r === "source" ? "→" : r === "target" ? "←" : "↔";
// A connection: its layer, ends (with ports), the other links sharing its
// wire, and its details.
function connectionHtml(c: Connection, editable: boolean) {
  const layer = doc.layers.find((l) => l.id === c.layer)!;
  const sharing = current?.bundles.find((b) => b.connections.includes(c.id));
  const others = (sharing?.connections || []).filter((id) => id !== c.id);
  return `<span class="eyebrow"><span class="swatch" style="background:${esc(layer.color)}"></span>CONNECTION · ${esc(layer.name.toUpperCase())}</span><h2>${esc(c.name)}</h2><div class="ends"><h3>Ends</h3>${c.endpoints
    .map((ep) => {
      const e = doc.entities.find((x) => x.id === ep.entity)!;
      return `<div class="end"><span class="role" title="${ep.role}">${{ source: "from", target: "to", peer: "with" }[ep.role]}</span><button class="link-button" data-select-entity="${esc(e.id)}">${esc(e.name)}</button>${ep.port ? `<button class="port-chip" data-select-port="${portRef(e.id, ep.port)}" title="Port ${esc(ep.port)}">${esc(ep.port)}</button>` : ""}</div>`;
    })
    .join(
      "",
    )}</div>${others.length ? `<div class="sharing"><h3>Same wire</h3>${others.map((id) => `<button class="link-button" data-select-connection="${esc(id)}">${esc(doc.connections.find((x) => x.id === id)!.name)}</button>`).join("")}</div>` : ""}<div class="focus-actions"><button data-focus-connection="${esc(c.id)}">Show only this</button></div>${detailsHtml(c)}${
    editable
      ? `<form id="connection-form"><label class="field">Name<input name="name" value="${esc(c.name)}" required maxlength="200"></label><label class="field">Layer<select name="layer">${doc.layers.map((l) => `<option value="${esc(l.id)}" ${l.id === c.layer ? "selected" : ""}>${esc(l.name)}</option>`).join("")}</select></label><div class="button-row"><button>Apply</button><button type="button" id="delete-connection" class="danger">Delete</button></div></form>${detailsEditorHtml(connectionTarget(c))}`
      : ""
  }`;
}
// A port: where it is, its rating, what it connects to, and its details.
function portHtml(e: Entity, p: Port, editable: boolean) {
  const mine = doc.connections.filter((c) =>
    c.endpoints.some((ep) => ep.entity === e.id && ep.port === p.id),
  );
  const facts = [
    p.main ? "Supply" : "",
    !topMain(p) && spaces(e, p)
      ? `${e.faceplate === "panel" ? "Space" : "Position"} ${spaces(e, p)}`
      : "",
    ratingText(p),
  ].filter(Boolean);
  return `<span class="eyebrow">PORT · <button class="link-button" data-select-entity="${esc(e.id)}">${esc(e.name.toUpperCase())}</button></span><h2>${esc(p.id)}${p.label ? ` <small>${esc(p.label)}</small>` : ""}</h2>${facts.length ? `<p class="muted">${esc(facts.join(" · "))}</p>` : ""}<div class="conn-list"><h3>Connections</h3>${
    mine.length
      ? mine
          .map((c) => {
            const layer = doc.layers.find((l) => l.id === c.layer)!;
            const to = c.endpoints
              .filter((ep) => ep.entity !== e.id)
              .map((ep) => esc(nameOf(ep.entity)))
              .join(", ");
            return `<div class="conn"><span class="swatch" style="background:${esc(layer.color)}"></span><div><button class="link-button" data-select-connection="${esc(c.id)}">${roleArrow(c.endpoints.find((ep) => ep.entity === e.id)!.role)} ${to}</button><small>${esc(c.name)}</small></div><span></span></div>`;
          })
          .join("")
      : '<p class="muted">Nothing connected</p>'
  }</div>${portLiveHtml(e, p)}${detailsHtml(p)}${editable ? detailsEditorHtml(portTarget(e, p)) : ""}`;
}
function renderConnectionInspector(): boolean {
  const c = doc.connections.find((c) => c.id === selectedConnection);
  if (!c) return false;
  const root = $("inspector");
  root.innerHTML = connectionHtml(c, writable);
  wireSelectors(root);
  wireDetailFilters(root);
  root.querySelector<HTMLButtonElement>("[data-focus-connection]")!.onclick =
    () =>
      openTrace({
        entities: c.endpoints.map((ep) => ep.entity),
        hops: 0,
        direction: "both",
      });
  if (writable) {
    wireDetailsEditor(connectionTarget(c));
    const form = $<HTMLFormElement>("connection-form");
    form.onsubmit = (ev) => {
      ev.preventDefault();
      const f = new FormData(form);
      const name = String(f.get("name")).trim();
      if (!name) return status("A connection needs a name", true);
      tryEdit((d) => {
        const x = d.connections.find((x) => x.id === c.id)!;
        x.name = name;
        x.layer = String(f.get("layer"));
      });
    };
    $("delete-connection").onclick = () => {
      if (!confirm(`Delete the connection “${c.name}”?`)) return;
      selectedConnection = null;
      tryEdit((d) => {
        d.connections = d.connections.filter((x) => x.id !== c.id);
      });
    };
  }
  return true;
}
function renderPortInspector(): boolean {
  const e = doc.entities.find((e) => e.id === selectedPort?.entity);
  const p = e?.ports?.find((p) => p.id === selectedPort?.port);
  if (!e || !p) return false;
  const root = $("inspector");
  root.innerHTML = portHtml(e, p, writable);
  wireSelectors(root);
  wireDetailFilters(root);
  if (writable) wireDetailsEditor(portTarget(e, p));
  return true;
}
function renderInspector() {
  if (selectedConnection) {
    if (renderConnectionInspector()) return;
    selectedConnection = null;
  }
  if (selectedPort) {
    if (renderPortInspector()) return;
    selectedPort = null;
  }
  const e = doc.entities.find((e) => e.id === selected);
  if (!e) {
    $("inspector").innerHTML =
      '<span class="eyebrow">DETAILS</span><h2>Every thing has a place.</h2><p class="muted">Select a card to inspect it.</p>';
    return;
  }
  const descendants = new Set([e.id]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const n of doc.entities)
      if (n.parent && descendants.has(n.parent) && !descendants.has(n.id)) {
        descendants.add(n.id);
        changed = true;
      }
  }
  const hasLayout = !!(e.ports?.length || e.faceplate);
  const leaf =
    !doc.entities.some((n) => n.parent === e.id) && e.kind !== "space";
  const kinds = [
    ...new Set([
      "space",
      "device",
      "equipment",
      "service",
      ...doc.entities.map((n) => n.kind),
    ]),
  ];
  $("inspector").innerHTML =
    `<span class="eyebrow">SELECTED ${esc(e.kind.toUpperCase())}</span><h2>${esc(e.name)}</h2><p class="muted">${doc.connections.filter((c) => c.endpoints.some((ep) => ep.entity === e.id)).length} connections · ${doc.entities.filter((n) => n.parent === e.id).length} children</p>${liveLinksHtml(e)}${actionsHtml(e)}${detailsHtml(e)}${scheduleHtml(e)}${connectionsHtml(e, writable)}${
      writable
        ? `<form id="entity-form"><label class="field">Name<input name="name" value="${esc(e.name)}" required maxlength="150"></label><label class="field">Type<input name="kind" list="kinds" value="${esc(e.kind)}" required maxlength="40"><datalist id="kinds">${kinds.map((k) => `<option value="${esc(k)}">`).join("")}</datalist></label><label class="field">Parent<select name="parent"><option value="">Top level</option>${doc.entities
            .filter((n) => !descendants.has(n.id))
            .map(
              (n) =>
                `<option value="${esc(n.id)}" ${e.parent === n.id ? "selected" : ""}>${esc(n.name)}</option>`,
            )
            .join(
              "",
            )}</select></label><div class="button-row"><button>Apply</button><button type="button" id="delete-entity" class="danger">Delete${descendants.size > 1 ? ` (${descendants.size})` : ""}</button></div></form>${detailsEditorHtml(entityTarget(e))}${leaf ? portEditorHtml(e) : ""}<form id="connect-form" class="connect-form"><h3>Connect this thing</h3><label class="field">Connection name<input name="name" required placeholder="e.g. Office power"></label><label class="field">Layer<select name="layer">${doc.layers.map((l) => `<option value="${esc(l.id)}">${esc(l.name)}</option>`).join("")}</select></label><label class="field">Other endpoints<select name="targets" multiple required size="4">${doc.entities
            .filter((n) => n.id !== e.id)
            .map((n) => `<option value="${esc(n.id)}">${esc(n.name)}</option>`)
            .join(
              "",
            )}</select></label><label class="field">Direction<select name="directed"><option value="yes">From this thing → targets</option><option value="no">Undirected peers</option></select></label><button>Add connection</button></form>`
        : '<p class="muted">Move cards, fold spaces, and explore layers freely.</p>'
    }`;
  const entityForm = $<HTMLFormElement>("entity-form");
  if (entityForm)
    entityForm.onsubmit = (event) => {
      event.preventDefault();
      const f = new FormData(entityForm);
      const name = String(f.get("name")).trim();
      if (!name) {
        status("An entity needs a name", true);
        return;
      }
      e.name = name;
      e.kind = String(f.get("kind")).trim() || e.kind;
      const parent = String(f.get("parent")) || null;
      if (e.parent !== parent) delete view.positions[e.id];
      e.parent = parent;
      markDirty();
    };
  const connect = $<HTMLFormElement>("connect-form");
  if (connect)
    connect.onsubmit = (event) => {
      event.preventDefault();
      const f = new FormData(connect);
      const directed = f.get("directed") === "yes";
      doc.connections.push({
        id: crypto.randomUUID(),
        name: String(f.get("name")),
        layer: String(f.get("layer")),
        endpoints: [
          { entity: e.id, role: directed ? "source" : "peer" },
          ...f.getAll("targets").map((id) => ({
            entity: String(id),
            role: (directed ? "target" : "peer") as Role,
          })),
        ],
      });
      markDirty();
    };
  const del = $("delete-entity");
  if (del)
    del.onclick = () => {
      doc.entities = doc.entities.filter((n) => !descendants.has(n.id));
      doc.connections = doc.connections
        .map((c) => ({
          ...c,
          endpoints: c.endpoints.filter((ep) => !descendants.has(ep.entity)),
        }))
        .filter((c) => c.endpoints.length >= 2);
      selected = "";
      markDirty();
    };
  wireEntityActions($("inspector"), e);
  if (writable) wireDetailsEditor(entityTarget(e));
  wireSelectors($("inspector"));
  $("inspector")
    .querySelectorAll<HTMLSelectElement>("[data-assign]")
    .forEach(
      (sel) =>
        (sel.onchange = () =>
          tryEdit((d) => {
            const c = d.connections.find((c) => c.id === sel.dataset.assign)!;
            const ep = c.endpoints.find((ep) => ep.entity === e.id)!;
            if (sel.value) ep.port = sel.value;
            else delete ep.port;
          })),
    );
  const portsForm = $<HTMLFormElement>("ports-form");
  if (portsForm) {
    const details = portsForm.closest("details")!;
    details.ontoggle = () => (portEditorOpen = details.open);
    const rows = (): (Port & { orig?: string })[] =>
      [...portsForm.querySelectorAll<HTMLElement>(".port-row[data-row]")].map(
        (tr) => {
          const v = (n: string) =>
            tr
              .querySelector<HTMLInputElement | HTMLSelectElement>(
                `[name=${n}]`,
              )!
              .value.trim();
          const p: Port & { orig?: string } = {
            id: v("id"),
            orig: tr.dataset.orig,
          };
          if (v("label")) p.label = v("label");
          const supply =
            tr.querySelector<HTMLInputElement>("[name=supply]")!.checked;
          if (/^main$/i.test(v("position"))) p.main = true;
          else {
            if (v("position")) p.position = Number(v("position"));
            if (supply) p.main = true;
          }
          if (Number(v("poles")) > 1) p.poles = Number(v("poles"));
          if (Number(v("amps")) > 0) p.amps = Number(v("amps"));
          if (v("protection"))
            p.protection = v("protection") as Port["protection"];
          const half = v("half").toUpperCase();
          if (half.includes("/"))
            p.halves = half.split("/").map((h) => (h === "-" ? "" : h));
          else if (half) p.half = half as "A" | "B";
          return p;
        },
      );
    const redraw = (ports: (Port & { orig?: string })[]) => {
      draftPorts = {
        entity: e.id,
        faceplate:
          (new FormData(portsForm).get("faceplate") as Faceplate) || undefined,
        ports,
      };
      renderInspector();
    };
    portsForm.querySelectorAll<HTMLButtonElement>(".remove-port").forEach(
      (b) =>
        (b.onclick = () => {
          const i = Number(b.closest<HTMLElement>(".port-row")!.dataset.row);
          redraw(rows().filter((_, k) => k !== i));
        }),
    );
    $("add-port").onclick = () => {
      const list = rows();
      const next =
        Math.max(
          0,
          ...list.map((p) => (p.position || 0) + (p.poles || 1) - 1),
        ) + 1;
      redraw([...list, { id: String(next), position: next }]);
    };
    $("number-ports").onclick = () => {
      const n = Number(prompt("Number of ports or panel spaces?", "24"));
      if (!Number.isInteger(n) || n < 1 || n > 256) return;
      const list = rows();
      const used = new Set(list.map((p) => p.position));
      for (let i = 1; i <= n; i++)
        if (!used.has(i) && !list.some((p) => p.id === String(i)))
          list.push({ id: String(i), position: i });
      redraw(list.sort(portOrder));
    };
    portsForm.onsubmit = (ev) => {
      ev.preventDefault();
      const list = rows();
      const faceplate =
        (new FormData(portsForm).get("faceplate") as Faceplate) || undefined;
      let unassigned = 0;
      const ok = tryEdit((d) => {
        const target = d.entities.find((x) => x.id === e.id)!;
        // Renamed ports carry their connections; removed ones release them.
        const renamed = new Map(
          list.filter((p) => p.orig).map((p) => [p.orig!, p.id]),
        );
        for (const c of d.connections)
          for (const ep of c.endpoints)
            if (ep.entity === e.id && ep.port) {
              const now =
                renamed.get(ep.port) ??
                (list.some((p) => p.id === ep.port) ? ep.port : undefined);
              if (now) ep.port = now;
              else {
                delete ep.port;
                unassigned++;
              }
            }
        target.ports = list.map(({ orig, ...p }) => p);
        if (!target.ports.length) delete target.ports;
        if (faceplate) target.faceplate = faceplate;
        else delete target.faceplate;
      });
      if (ok) {
        draftPorts = null;
        if (unassigned)
          status(
            `${unassigned} connection(s) lost their port and are now unassigned`,
          );
      }
    };
  }
}
// Details: key/value properties. Text shows as text, links open in a new
// tab, images as thumbnails, other files (PDF manuals) as downloads.
const attachmentUrl = (id: string) => `api/attachments/${id}`;
const isPicture = (mime: string) =>
  mime.startsWith("image/") && !mime.includes("svg");
const sizeText = (n: number) =>
  n < 1024
    ? `${n} B`
    : n < 1 << 20
      ? `${Math.round(n / 1024)} KB`
      : `${(n / (1 << 20)).toFixed(1)} MB`;
function detailsHtml(e: { properties?: Property[] }) {
  if (!e.properties?.length) return "";
  const value = (p: Property) => {
    if (p.type === "text")
      return `<span class="detail-text">${esc(p.text)}</span><button class="same" data-same-key="${esc(p.key)}" data-same-value="${esc(p.text)}" title="Show everything whose ${esc(p.key)} is ${esc(p.text)}" aria-label="Show everything with this ${esc(p.key)}">⧉</button>`;
    if (p.type === "link")
      return `<a href="${esc(p.url)}" target="_blank" rel="noopener noreferrer">${esc(p.title || p.url)}</a>`;
    // The offline export carries no attachments, only their names.
    if (offline)
      return `<span class="muted">${esc(p.name)} (not in this export)</span>`;
    const url = attachmentUrl(p.attachment);
    return isPicture(p.mime)
      ? `<a href="${url}" target="_blank" rel="noopener" class="detail-image"><img src="${url}" alt="${esc(p.name)}" loading="lazy"></a>`
      : `<a href="${url}" target="_blank" rel="noopener" class="detail-file">📄 ${esc(p.name)} <small>${sizeText(p.size)}</small></a>`;
  };
  return `<div class="details-list"><h3>Details</h3><dl>${e.properties
    .map(
      (p) =>
        `<dt><button class="same-key" data-has-key="${esc(p.key)}" title="Show everything with a ${esc(p.key)}">${esc(p.key)}</button></dt><dd>${value(p)}</dd>`,
    )
    .join("")}</dl></div>`;
}
// Details are drafted in the editor until applied, like ports.
// What a details editor edits: a thing, a connection, or a port.
type DetailTarget = {
  key: string;
  props?: Property[];
  apply: (d: DocumentData, props?: Property[]) => void;
};
const entityTarget = (e: Entity): DetailTarget => ({
  key: `e:${e.id}`,
  props: e.properties,
  apply: (d, props) => {
    const x = d.entities.find((x) => x.id === e.id)!;
    if (props) x.properties = props;
    else delete x.properties;
  },
});
const connectionTarget = (c: Connection): DetailTarget => ({
  key: `c:${c.id}`,
  props: c.properties,
  apply: (d, props) => {
    const x = d.connections.find((x) => x.id === c.id)!;
    if (props) x.properties = props;
    else delete x.properties;
  },
});
const portTarget = (e: Entity, port: Port): DetailTarget => ({
  key: `p:${e.id}:${port.id}`,
  props: port.properties,
  apply: (d, props) => {
    const x = d.entities
      .find((x) => x.id === e.id)!
      .ports!.find((p) => p.id === port.id)!;
    if (props) x.properties = props;
    else delete x.properties;
  },
});
let draftDetails: { key: string; rows: Property[] } | null = null;
let detailsEditorOpen = false;
function detailsEditorHtml(t: DetailTarget) {
  const rows =
    draftDetails?.key === t.key
      ? draftDetails.rows
      : structuredClone(t.props || []);
  draftDetails = { key: t.key, rows };
  const valueInputs = (p: Property, i: number) =>
    p.type === "text"
      ? `<input data-detail="${i}" data-field="text" value="${esc(p.text)}" maxlength="4000" placeholder="Value" aria-label="Value">`
      : p.type === "link"
        ? `<input data-detail="${i}" data-field="url" type="url" value="${esc(p.url)}" placeholder="https://…" aria-label="Link"><input data-detail="${i}" data-field="title" value="${esc(p.title || "")}" maxlength="200" placeholder="Title (optional)" aria-label="Link title">`
        : `<div class="detail-upload">${p.attachment ? `<span>${esc(p.name)} <small>${sizeText(p.size)}</small></span>` : '<span class="muted">No file yet</span>'}<input data-detail="${i}" data-field="file" type="file" aria-label="Choose file"></div>`;
  return `<details class="details-editor" ${detailsEditorOpen ? "open" : ""}><summary>Edit details</summary><div id="details-rows">${rows
    .map(
      (p, i) =>
        `<div class="detail-row"><div class="l1"><input data-detail="${i}" data-field="key" value="${esc(p.key)}" maxlength="80" placeholder="Key, e.g. IP address" aria-label="Key"><select data-detail="${i}" data-field="type" aria-label="Kind">${(["text", "link", "file"] as const).map((t) => `<option value="${t}" ${p.type === t ? "selected" : ""}>${{ text: "Text", link: "Link", file: "File" }[t]}</option>`).join("")}</select><button type="button" class="remove-port" data-remove-detail="${i}" aria-label="Remove ${esc(p.key || "detail")}">✕</button></div><div class="l2">${valueInputs(p, i)}</div></div>`,
    )
    .join(
      "",
    )}</div><div class="button-row"><button type="button" id="add-detail">＋ Detail</button><button type="button" id="apply-details">Apply details</button></div></details>`;
}
function wireDetailsEditor(t: DetailTarget) {
  const root = document.querySelector<HTMLDetailsElement>(
    "#inspector .details-editor",
  );
  if (!root || !draftDetails) return;
  const rows = draftDetails.rows;
  root.ontoggle = () => (detailsEditorOpen = root.open);
  root
    .querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-detail]")
    .forEach((el) => {
      const i = Number(el.dataset.detail),
        field = el.dataset.field!;
      if (field === "file")
        el.onchange = async () => {
          const file = (el as HTMLInputElement).files?.[0];
          if (!file) return;
          status(`Uploading ${file.name}…`);
          try {
            const r = await fetch("api/attachments", {
              method: "POST",
              headers: {
                "Content-Type": file.type || "application/octet-stream",
                "X-File-Name": encodeURIComponent(file.name),
              },
              body: file,
            });
            if (!r.ok) throw new Error(await r.text());
            const a = await r.json();
            rows[i] = { key: rows[i].key || file.name, type: "file", ...a };
            status(`Uploaded ${file.name}; apply and save to keep it`);
          } catch (err) {
            status(`Upload failed: ${(err as Error).message}`, true);
          }
          renderInspector();
        };
      else if (field === "type")
        el.onchange = () => {
          const key = rows[i].key;
          const t = el.value as Property["type"];
          rows[i] =
            t === "text"
              ? { key, type: "text", text: "" }
              : t === "link"
                ? { key, type: "link", url: "", title: "" }
                : {
                    key,
                    type: "file",
                    attachment: "",
                    name: "",
                    mime: "",
                    size: 0,
                  };
          renderInspector();
        };
      else
        el.oninput = () =>
          ((rows[i] as Record<string, unknown>)[field] = el.value);
    });
  root.querySelectorAll<HTMLButtonElement>("[data-remove-detail]").forEach(
    (b) =>
      (b.onclick = () => {
        rows.splice(Number(b.dataset.removeDetail), 1);
        renderInspector();
      }),
  );
  $("add-detail").onclick = () => {
    rows.push({ key: "", type: "text", text: "" });
    renderInspector();
  };
  $("apply-details").onclick = () => {
    const keep = rows
      .map((p) => ({ ...p, key: p.key.trim() }))
      .filter((p) => p.key && !(p.type === "file" && !p.attachment));
    if (tryEdit((d) => t.apply(d, keep.length ? keep : undefined))) {
      draftDetails = null;
      status("Details applied; save to keep them");
    }
  };
}
// The view actions for one thing: show it, trace it, draw its faceplate.
// Shared by the inspector and the phone's bottom sheet.
function actionsHtml(e: Entity) {
  const leaf =
    !doc.entities.some((n) => n.parent === e.id) && e.kind !== "space";
  const hasLayout = !!(e.ports?.length || e.faceplate);
  return `<div class="focus-actions"><button data-focus="both" title="Show this and what it connects to">Show only this</button><button data-focus="upstream" title="Trace everything that feeds this">↑ What feeds this</button><button data-focus="downstream" title="Trace everything this feeds">↓ What this feeds</button></div>${
    hasLayout && leaf
      ? `<label class="toggle"><input type="checkbox" class="faceplate-toggle" ${view.faceplates?.includes(e.id) ? "checked" : ""}> Show physical layout</label>`
      : ""
  }`;
}
function wireEntityActions(root: HTMLElement, e: Entity) {
  root.querySelectorAll<HTMLButtonElement>("[data-focus]").forEach(
    (b) =>
      (b.onclick = () => {
        const direction = b.dataset.focus as Direction;
        sheetId = null;
        openTrace({
          entities: [e.id],
          hops: direction === "both" ? 1 : undefined,
          direction,
        });
      }),
  );
  const toggle = root.querySelector<HTMLInputElement>(".faceplate-toggle");
  if (toggle)
    toggle.onchange = () => {
      const set = new Set(view.faceplates || []);
      if (toggle.checked) set.add(e.id);
      else set.delete(e.id);
      view.faceplates = [...set];
      if (toggle.checked) settle(e.id);
      render();
    };
  const print = root.querySelector<HTMLButtonElement>(".print-schedule");
  if (print) print.onclick = () => printSchedule(e);
  wireDetailFilters(root);
}
function wireDetailFilters(root: HTMLElement) {
  root.querySelectorAll<HTMLButtonElement>("[data-same-key]").forEach(
    (b) =>
      (b.onclick = () => {
        sheetId = null;
        showMatching({
          conditions: [
            {
              key: b.dataset.sameKey!,
              op: "equals",
              value: b.dataset.sameValue!,
            },
          ],
        });
      }),
  );
  root.querySelectorAll<HTMLButtonElement>("[data-has-key]").forEach(
    (b) =>
      (b.onclick = () => {
        sheetId = null;
        showMatching({
          conditions: [{ key: b.dataset.hasKey!, op: "exists" }],
        });
      }),
  );
}
// Phones have no inspector column: tapping a thing opens this sheet instead.
type SheetTarget =
  | { kind: "entity"; id: string }
  | { kind: "connection"; id: string }
  | { kind: "port"; id: string; port: string };
let sheetTarget: SheetTarget | null = null;
// The entity a sheet shows, for the code paths that only handle things.
let sheetId: string | null = null;
function openSheet(t: SheetTarget) {
  sheetTarget = t;
  sheetId = t.kind === "entity" ? t.id : null;
  renderSheet();
}
const narrow = () => matchMedia("(max-width: 800px)").matches;
function renderSheet() {
  const sheet = $("sheet");
  if (!sheetId && sheetTarget?.kind === "entity") sheetTarget = null;
  const t = sheetTarget;
  const close = `<button class="sheet-close" aria-label="Close">✕</button>`;
  const wireClose = () =>
    (sheet.querySelector<HTMLButtonElement>(".sheet-close")!.onclick = () => {
      sheetId = null;
      sheetTarget = null;
      renderSheet();
    });
  if (t?.kind === "connection") {
    const c = doc.connections.find((c) => c.id === t.id);
    sheet.hidden = !c;
    if (!c) return;
    sheet.innerHTML = `<div class="sheet-head"><div>${connectionHtml(c, false)}</div>${close}</div>`;
    wireClose();
    wireSelectors(sheet, true);
    wireDetailFilters(sheet);
    sheet.querySelector<HTMLButtonElement>("[data-focus-connection]")!.onclick =
      () => {
        sheetTarget = null;
        renderSheet();
        openTrace({
          entities: c.endpoints.map((ep) => ep.entity),
          hops: 0,
          direction: "both",
        });
      };
    return;
  }
  if (t?.kind === "port") {
    const e = doc.entities.find((x) => x.id === t.id);
    const p = e?.ports?.find((p) => p.id === t.port);
    sheet.hidden = !p;
    if (!e || !p) return;
    sheet.innerHTML = `<div class="sheet-head"><div>${portHtml(e, p, false)}</div>${close}</div>`;
    wireClose();
    wireSelectors(sheet, true);
    wireDetailFilters(sheet);
    return;
  }
  const e = sheetId ? doc.entities.find((x) => x.id === sheetId) : undefined;
  sheet.hidden = !e;
  if (!e) return;
  const inside = doc.entities.filter((n) => n.parent === e.id).length;
  const links = doc.connections.filter((c) =>
    c.endpoints.some((ep) => ep.entity === e.id),
  ).length;
  sheet.innerHTML = `<div class="sheet-head"><div><span class="eyebrow">${esc(e.kind.toUpperCase())}</span><h2>${esc(e.name)}</h2><p class="muted">${links} connections${inside ? ` · ${inside} inside` : ""}</p></div><button class="sheet-close" aria-label="Close">✕</button></div>${liveLinksHtml(e)}${actionsHtml(e)}${detailsHtml(e)}${scheduleHtml(e)}${connectionsHtml(e, false)}`;
  sheet.querySelector<HTMLButtonElement>(".sheet-close")!.onclick = () => {
    sheetId = null;
    renderSheet();
  };
  wireEntityActions(sheet, e);
  wireSelectors(sheet, true);
}
// Select mode: on touch there is no shift key, so taps pick and a drag lassos.
let selectMode = false;
function setSelectMode(on: boolean) {
  selectMode = on;
  const b = $("select-mode");
  b.classList.toggle("on", on);
  b.setAttribute("aria-pressed", String(on));
  if (on) {
    sheetId = null;
    renderSheet();
  }
  renderBars();
}
// A faceplate is larger than the card it replaces: re-drop it where it is
// so its neighbours are pushed aside (in the full view, even mid-focus).
function settle(id: string) {
  const full = { ...view, focus: undefined };
  const at: Layout = JSON.parse(
    preview(JSON.stringify({ document: doc, view: full })),
  );
  const n = at.nodes.find((n) => n.id === id);
  if (!n) return;
  const moved: View = JSON.parse(
    move_node(
      JSON.stringify({
        document: doc,
        view: full,
        entity: id,
        position: n.position,
        dragging: false,
      }),
    ),
  );
  view = { ...moved, focus: view.focus };
  if (!view.focus) {
    delete view.focus;
    return;
  }
  // ...and inside the focus, whose own arrangement it also grew in.
  const inFocus: Layout = JSON.parse(
    preview(JSON.stringify({ document: doc, view })),
  );
  const m = inFocus.nodes.find((n) => n.id === id);
  if (m)
    view = JSON.parse(
      move_node(
        JSON.stringify({
          document: doc,
          view,
          entity: id,
          position: m.position,
          dragging: false,
        }),
      ),
    );
}
// Port edits are drafted in the form until applied.
let draftPorts: {
  entity: string;
  faceplate?: Faceplate;
  ports: (Port & { orig?: string })[];
} | null = null;
// Apply an inventory edit only if the Rust engine accepts the result.
function tryEdit(change: (d: DocumentData) => void) {
  const next = structuredClone(doc);
  change(next);
  try {
    compute(JSON.stringify({ document: next, view }));
  } catch (err) {
    status(String((err as Error).message || err), true);
    renderInspector();
    return false;
  }
  doc = next;
  markDirty();
  return true;
}
function printSchedule(e: Entity) {
  const { rows, unassigned } = schedule(e);
  const cell = (list: { c: Connection; to: string[] }[]) =>
    list
      .map(
        ({ c, to }) => `${esc(to.join(", "))}<br><small>${esc(c.name)}</small>`,
      )
      .join("<hr>") || "spare";
  const w = window.open("", "_blank");
  if (!w) return status("Allow pop-ups to print the schedule", true);
  w.document.write(
    `<!doctype html><meta charset="utf-8"><title>${esc(e.name)}</title><style>body{font:13px system-ui,sans-serif;margin:24px}table{border-collapse:collapse;width:100%}td,th{border:1px solid #999;padding:4px 6px;text-align:left;vertical-align:top}small{color:#555}hr{border:0;border-top:1px dotted #bbb;margin:3px 0}</style><h1>${esc(e.name)}</h1><table><tr><th>${e.faceplate === "panel" ? "Space" : "#"}</th><th>Port</th><th>Rating</th><th>Label</th><th>Connected to</th></tr>${rows
      .map(
        (r) =>
          `<tr><td>${esc(r.position)}</td><td>${esc(r.port.id)}</td><td>${esc(ratingText(r.port))}</td><td>${esc(r.port.label || "")}</td><td>${cell(r.connections)}</td></tr>`,
      )
      .join(
        "",
      )}${unassigned.length ? `<tr><td>?</td><td colspan=3>Not assigned to a port</td><td>${cell(unassigned)}</td></tr>` : ""}</table><script>print()<\/script>`,
  );
  w.document.close();
}
function markDirty() {
  dirty = true;
  render();
  status("Unsaved inventory changes");
}
function fold(id: string) {
  view.collapsed = view.collapsed.includes(id)
    ? view.collapsed.filter((v) => v !== id)
    : [...view.collapsed, id];
  render();
}
function applyCamera() {
  document
    .getElementById("scene")!
    .setAttribute(
      "transform",
      `translate(${camera.x},${camera.y}) scale(${camera.scale})`,
    );
  $("zoom").textContent = `${Math.round(camera.scale * 100)}%`;
}
function fit() {
  if (mode === "list" || !current) return;
  const box = $("canvas").getBoundingClientRect();
  const ps = [
    ...current.nodes.flatMap((n) => [
      n.position,
      { x: n.position.x + n.width, y: n.position.y + n.height },
    ]),
    ...current.bundles.flatMap((b) => [
      ...b.junctions,
      ...b.branches.flatMap((branch) => branch.points),
    ]),
  ];
  if (!ps.length) return;
  const minX = Math.min(...ps.map((p) => p.x)) - 40,
    minY = Math.min(...ps.map((p) => p.y)) - 40;
  const maxX = Math.max(...ps.map((p) => p.x)) + 40,
    maxY = Math.max(...ps.map((p) => p.y)) + 80;
  // Keep clear of the focus bar floating over the top of the canvas.
  const top = view.focus ? ($("focus-bar").offsetHeight || 40) + 24 : 20;
  camera.scale = Math.min(
    1.2,
    (box.width - 30) / (maxX - minX),
    (box.height - top - 20) / (maxY - minY),
  );
  camera.x =
    (box.width - (maxX - minX) * camera.scale) / 2 - minX * camera.scale;
  camera.y = top - minY * camera.scale;
  applyCamera();
}
function zoom(mult: number, x?: number, y?: number) {
  const r = svg.getBoundingClientRect();
  x ??= r.width / 2;
  y ??= r.height / 2;
  const scale = Math.max(0.15, Math.min(2.5, camera.scale * mult));
  const ratio = scale / camera.scale;
  camera.x = x - (x - camera.x) * ratio;
  camera.y = y - (y - camera.y) * ratio;
  camera.scale = scale;
  applyCamera();
}
let drag: { id: string | null; start: Point; origin: Point } | null = null;
// A drag is computed from the view it started with: every frame moves the
// dragged box from there (so boxes it pushed or swapped spring back as it moves
// on) and draws a routing-free preview, cheap enough for every animation
// frame. On drop the final move lands and one full routing pass runs, in a
// worker when one is available so the page stays responsive.
let dragBase: View | null = null;
let latestMove: { id: string; position: Point } | null = null;
let moveFrame = 0;
let layoutWorker: Worker | null = null,
  // Bumped whenever the view changes after a full layout was requested; a
  // worker reply for an older view is then discarded.
  layoutGeneration = 0;
function moveFrom(base: View, dragging: boolean) {
  view = JSON.parse(
    move_node(
      JSON.stringify({
        document: doc,
        view: base,
        entity: latestMove!.id,
        position: latestMove!.position,
        dragging,
      }),
    ),
  );
  applyLayout(JSON.parse(preview(JSON.stringify({ document: doc, view }))));
  renderScene();
}
function beginDrag() {
  finishDrag();
  dragBase = structuredClone(view);
  layoutGeneration++;
}
function requestMove(move: { id: string; position: Point }) {
  latestMove = move;
  if (!moveFrame)
    moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      if (dragBase && latestMove) moveFrom(dragBase, true);
    });
}
function finishDrag() {
  if (moveFrame) cancelAnimationFrame(moveFrame);
  moveFrame = 0;
  const base = dragBase;
  dragBase = null;
  if (!base || !latestMove) return;
  moveFrom(base, false);
  latestMove = null;
  routeInBackground();
}
// `data-layout-pending` marks a drawn preview awaiting its routed layout, so
// automation can wait for the settled scene.
function routeInBackground() {
  const generation = ++layoutGeneration;
  if (!layoutWorker) {
    computeLayout();
    renderScene();
    return;
  }
  svg.dataset.layoutPending = "";
  layoutWorker.postMessage({
    generation,
    input: JSON.stringify({ document: doc, view }),
  });
}
function startLayoutWorker(module: WebAssembly.Module) {
  let worker: Worker;
  try {
    const url = URL.createObjectURL(
      new Blob([WORKER_SOURCE], { type: "text/javascript" }),
    );
    worker = new Worker(url);
    URL.revokeObjectURL(url);
    worker.postMessage({ module });
  } catch {
    return; // e.g. a sandboxed export; route on this thread instead
  }
  const fallBack = () => {
    worker.terminate();
    layoutWorker = null;
    if ("layoutPending" in svg.dataset) {
      delete svg.dataset.layoutPending;
      routeInBackground();
    }
  };
  worker.onerror = fallBack;
  worker.onmessage = (ev) => {
    const r = ev.data;
    if (r.error) {
      status(`Layout worker failed, routing in page: ${r.error}`, true);
      fallBack();
      return;
    }
    if (r.generation !== layoutGeneration) return;
    delete svg.dataset.layoutPending;
    applyLayout(JSON.parse(r.layout));
    renderScene();
  };
  layoutWorker = worker;
}
const pointers = new Map<number, Point>();
let pinch: { distance: number; scale: number; world: Point } | null = null;
function pointerPoint(ev: PointerEvent): Point {
  const r = svg.getBoundingClientRect();
  return { x: ev.clientX - r.left, y: ev.clientY - r.top };
}
function pinchGeometry() {
  const [a, b] = [...pointers.values()];
  return {
    center: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 },
    distance: Math.max(1, Math.hypot(a.x - b.x, a.y - b.y)),
  };
}
function startPinch() {
  tap = null;
  const { center, distance } = pinchGeometry();
  finishDrag();
  drag = null;
  pinch = {
    distance,
    scale: camera.scale,
    world: {
      x: (center.x - camera.x) / camera.scale,
      y: (center.y - camera.y) / camera.scale,
    },
  };
}
const distance = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y);
function endPointer(ev: PointerEvent) {
  if (!pointers.delete(ev.pointerId)) return;
  finishLasso();
  // A tap on a faceplate's port or on a wire inspects that port or wire.
  if (tap && !pointers.size && ev.type === "pointerup") {
    if (tap.port) selectedPort = { entity: tap.id, port: tap.port };
    if (tap.connection) {
      selectedConnection = tap.connection;
      selectedPort = null;
    }
    if (tap.port || tap.connection) {
      renderScene();
      renderInspector();
    }
    if (narrow())
      openSheet(
        tap.connection
          ? { kind: "connection", id: tap.connection }
          : tap.port
            ? { kind: "port", id: tap.id, port: tap.port }
            : { kind: "entity", id: tap.id },
      );
  }
  tap = null;
  // Land the drag where the pointer let go.
  finishDrag();
  if (pinch) {
    if (pointers.size >= 2) startPinch();
    else {
      pinch = null;
      const remaining = [...pointers.values()][0];
      // After a pinch, the remaining finger pans; it never resumes a node drag.
      drag = remaining
        ? { id: null, start: remaining, origin: { x: camera.x, y: camera.y } }
        : null;
    }
  } else drag = null;
}
svg.addEventListener("pointerdown", (ev) => {
  if (ev.button !== 0) return;
  ev.preventDefault();
  pointers.set(ev.pointerId, pointerPoint(ev));
  svg.setPointerCapture(ev.pointerId);
  if (pointers.size >= 2) {
    startPinch();
    return;
  }
  const target = ev.target as Element;
  const foldTarget = target.closest("[data-fold]");
  if (foldTarget) {
    fold(foldTarget.getAttribute("data-fold")!);
    return;
  }
  let id = target.closest("[data-id]")?.getAttribute("data-id") || null;
  const r = svg.getBoundingClientRect();
  // Shift (or Select mode, on touch) picks: a tap toggles a thing, dragging
  // the canvas draws a lasso.
  tap = null;
  if (ev.shiftKey || selectMode) {
    if (id) {
      if (picked.has(id)) picked.delete(id);
      else picked.add(id);
      renderScene();
      renderBars();
    } else lasso = { start: pointerPoint(ev), end: pointerPoint(ev) };
    return;
  }
  const wire = target.closest("[data-connections]");
  const slot = target.closest("[data-port]");
  tap =
    id || wire
      ? {
          id: id || "",
          at: pointerPoint(ev),
          port: id && slot ? slot.getAttribute("data-port")! : undefined,
          connection:
            !id && wire
              ? wire.getAttribute("data-connections")!.split(" ")[0]
              : undefined,
        }
      : null;
  if (id) {
    selectEntity(id);
    renderTree();
    renderInspector();
    document
      .querySelectorAll(".node.selected")
      .forEach((n) => n.classList.remove("selected"));
    target.closest(".node")?.classList.add("selected");
    const node = current.nodes.find((n) => n.id === id)!;
    if (node.container && !node.collapsed && !target.closest(".space-header"))
      id = null;
  }
  if (id) beginDrag();
  drag = {
    id,
    start: { x: ev.clientX - r.left, y: ev.clientY - r.top },
    origin: id
      ? { ...current.nodes.find((n) => n.id === id)!.position }
      : { x: camera.x, y: camera.y },
  };
  svg.setPointerCapture(ev.pointerId);
});
let lasso: { start: Point; end: Point } | null = null;
// A press on a thing that ends where it began (no drag) is a tap.
let tap: { id: string; at: Point; port?: string; connection?: string } | null =
  null;
function drawLasso() {
  const el = document.getElementById("lasso")!;
  if (!lasso) return el.setAttribute("visibility", "hidden");
  const { start: a, end: b } = lasso;
  el.setAttribute("x", String(Math.min(a.x, b.x)));
  el.setAttribute("y", String(Math.min(a.y, b.y)));
  el.setAttribute("width", String(Math.abs(a.x - b.x)));
  el.setAttribute("height", String(Math.abs(a.y - b.y)));
  el.setAttribute("visibility", "visible");
}
// Pick every thing (not the spaces enclosing it) the lasso touches.
function finishLasso() {
  if (!lasso) return;
  const toWorld = (p: Point) => ({
    x: (p.x - camera.x) / camera.scale,
    y: (p.y - camera.y) / camera.scale,
  });
  const a = toWorld(lasso.start),
    b = toWorld(lasso.end);
  const [x0, x1, y0, y1] = [
    Math.min(a.x, b.x),
    Math.max(a.x, b.x),
    Math.min(a.y, b.y),
    Math.max(a.y, b.y),
  ];
  if (x1 - x0 > 2 || y1 - y0 > 2)
    for (const n of current.nodes)
      if (
        (!n.container || n.collapsed) &&
        n.position.x < x1 &&
        n.position.x + n.width > x0 &&
        n.position.y < y1 &&
        n.position.y + n.height > y0
      )
        picked.add(n.id);
  lasso = null;
  drawLasso();
  renderScene();
  renderBars();
}
svg.addEventListener("pointermove", (ev) => {
  if (!pointers.has(ev.pointerId)) return;
  pointers.set(ev.pointerId, pointerPoint(ev));
  if (lasso) {
    lasso.end = pointerPoint(ev);
    drawLasso();
    return;
  }
  if (tap && distance(pointerPoint(ev), tap.at) > 8) tap = null;
  if (pinch) {
    const { center, distance } = pinchGeometry();
    camera.scale = Math.max(
      0.15,
      Math.min(2.5, (pinch.scale * distance) / pinch.distance),
    );
    camera.x = center.x - pinch.world.x * camera.scale;
    camera.y = center.y - pinch.world.y * camera.scale;
    applyCamera();
    return;
  }
  if (!drag) return;
  const r = svg.getBoundingClientRect();
  const dx = ev.clientX - r.left - drag.start.x,
    dy = ev.clientY - r.top - drag.start.y;
  if (drag.id) {
    requestMove({
      id: drag.id,
      position: {
        x: drag.origin.x + dx / camera.scale,
        y: drag.origin.y + dy / camera.scale,
      },
    });
  } else {
    camera.x = drag.origin.x + dx;
    camera.y = drag.origin.y + dy;
    applyCamera();
  }
});
svg.addEventListener("pointerup", endPointer);
svg.addEventListener("pointercancel", endPointer);
svg.addEventListener("lostpointercapture", endPointer);
svg.addEventListener("dblclick", (ev) => {
  const id = (ev.target as Element)
    .closest("[data-id]")
    ?.getAttribute("data-id");
  if (id) fold(id);
});
svg.addEventListener(
  "wheel",
  (ev) => {
    ev.preventDefault();
    const r = svg.getBoundingClientRect();
    zoom(
      Math.exp(-ev.deltaY * 0.0015),
      ev.clientX - r.left,
      ev.clientY - r.top,
    );
  },
  { passive: false },
);
$("tree").onclick = (ev) => {
  const t = ev.target as Element;
  const f = t.closest("[data-fold]");
  if (f) {
    fold(f.getAttribute("data-fold")!);
    return;
  }
  const row = t.closest("[data-select]");
  if (row) {
    selectEntity(row.getAttribute("data-select")!);
    render();
  }
};
$("layers").onchange = (ev) => {
  const t = ev.target as HTMLInputElement;
  const id = t.dataset.layer!;
  view.hidden_layers = t.checked
    ? view.hidden_layers.filter((l) => l !== id)
    : [...view.hidden_layers, id];
  render();
};
$("search").oninput = () => {
  renderTree();
  $("search-show").hidden = !$<HTMLInputElement>("search").value.trim();
};
$("search-show").onclick = () =>
  showMatching({
    conditions: [
      {
        key: "@any",
        op: "contains",
        value: $<HTMLInputElement>("search").value.trim(),
      },
    ],
  });
$("filter").onclick = () =>
  filterDraft ? ((filterDraft = null), renderFilterPanel()) : openFilterPanel();
$("select-mode").onclick = () => setSelectMode(!selectMode);
$("fit").onclick = fit;
$("zoom-in").onclick = () => zoom(1.2);
$("zoom-out").onclick = () => zoom(1 / 1.2);
$("add").onclick = () => {
  if (!writable) return;
  const id = crypto.randomUUID();
  doc.entities.push({
    id,
    name: "New thing",
    kind: "device",
    parent: selected || null,
    links: [],
  });
  if (selected) view.collapsed = view.collapsed.filter((i) => i !== selected);
  selected = id;
  markDirty();
  $<HTMLInputElement>("entity-form")
    ?.querySelector<HTMLInputElement>("input")
    ?.select();
};
$("save-document").onclick = run(async () => {
  const result = await api("document", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ revision, document: doc }),
  });
  revision = result.revision;
  dirty = false;
  render();
  status("Inventory saved to SQLite");
});
function updateViews() {
  $("views").innerHTML =
    '<option value="">Unsaved view</option>' +
    savedViews
      .map((v) => `<option value="${esc(v.id)}">${esc(v.name)}</option>`)
      .join("");
}
$("views").onchange = () => {
  const v = savedViews.find(
    (v) => v.id === $<HTMLSelectElement>("views").value,
  );
  if (v) {
    view = structuredClone(v.view);
    mode = "diagram";
    traceOpen = false;
    before = null;
    render();
    fit();
    status(`Loaded ${v.name}`);
  }
};
$("save-view").onclick = () => {
  const selectedView = savedViews.find(
    (v) => v.id === $<HTMLSelectElement>("views").value,
  );
  $<HTMLInputElement>("view-name").value = selectedView?.name || "";
  $<HTMLDialogElement>("view-dialog").showModal();
};
$("cancel-view").onclick = () => $<HTMLDialogElement>("view-dialog").close();
$("view-form").onsubmit = (ev) => {
  ev.preventDefault();
  run(async () => {
    const existing = savedViews.find(
      (v) => v.id === $<HTMLSelectElement>("views").value,
    );
    const saved = {
      id: existing?.id || crypto.randomUUID(),
      name: $<HTMLInputElement>("view-name").value,
      view,
    };
    await api("views", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(saved),
    });
    savedViews = await api("views");
    updateViews();
    $<HTMLSelectElement>("views").value = saved.id;
    $<HTMLDialogElement>("view-dialog").close();
    status("View saved to SQLite");
  })();
};
$("export").onclick = () => {
  const clone = document.documentElement.cloneNode(true) as HTMLElement;
  clone.querySelector("#snapshot")!.textContent = JSON.stringify({
    document: doc,
    view,
  }).replaceAll("<", "\\u003c");
  clone
    .querySelectorAll("dialog[open]")
    .forEach((d) => d.removeAttribute("open"));
  const blob = new Blob(["<!doctype html>\n", clone.outerHTML], {
    type: "text/html",
  });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = "stufftracker-view.html";
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  status("Exported self-contained viewer");
};
window.addEventListener("beforeunload", (ev) => {
  if (dirty) {
    ev.preventDefault();
    ev.returnValue = "";
  }
});
async function start() {
  const module = await WebAssembly.compile(
    Uint8Array.from(atob(WASM_BASE64), (c) => c.charCodeAt(0)),
  );
  await init({ module_or_path: module });
  startLayoutWorker(module);
  const snapshot = JSON.parse($("snapshot").textContent || "null");
  if (snapshot) {
    offline = true;
    doc = snapshot.document;
    view = snapshot.view;
    $("mode").textContent = "Interactive snapshot · No server required";
  } else {
    const data = await api("document");
    doc = data.document;
    revision = data.revision;
    writable = (await api("session")).writable;
    savedViews = await api("views");
  }
  $<HTMLButtonElement>("add").disabled = !writable;
  $<HTMLButtonElement>("save-view").disabled = !writable;
  $<HTMLSelectElement>("views").disabled = offline;
  $("save-document").hidden = !writable;
  updateViews();
  mode = offline || location.hash === "#everything" ? "diagram" : "list";
  render();
  // Status loads afterwards, on its own; drawing never waits for it.
  startLive();
  status(
    offline
      ? "Offline viewer · Rust/WASM engine"
      : `Rust/WASM engine · ${writable ? "SQLite connected" : "Read-only share"}`,
  );
}
run(start)();
$("to-list").onclick = () => {
  mode = "list";
  render();
};
$<HTMLInputElement>("areas").onchange = () => {
  if ($<HTMLInputElement>("areas").checked) view.areas = true;
  else delete view.areas;
  render();
  fit();
};
// Lay the current diagram (or focus) out afresh by its connections.
$("rearrange").onclick = () => {
  if (view.focus) view.focus.positions = {};
  else view.flat_positions = {};
  render();
  fit();
  status("Re-arranged by connections");
};
