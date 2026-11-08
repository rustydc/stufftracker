import init, { compute, move_node, preview } from "./pkg/stufftracker_core";
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
};
type Entity = {
  id: string;
  name: string;
  kind: string;
  parent: string | null;
  links: { provider: string; kind: string; external_id: string }[];
  faceplate?: Faceplate;
  ports?: Port[];
};
type Role = "source" | "target" | "peer";
type Endpoint = { entity: string; role: Role; port?: string };
type Connection = {
  id: string;
  name: string;
  layer: string;
  endpoints: Endpoint[];
};
type Direction = "both" | "upstream" | "downstream";
type Focus = { entities: string[]; hops?: number; direction?: Direction };
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
};
type Layout = {
  attachment_hints: Record<string, Point>;
  layout_version: number;
  unrouted_connections: string[];
  positions: Record<string, Point>;
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
}
function setFocus(focus: Focus | null) {
  if (focus) view.focus = focus;
  else delete view.focus;
  picked.clear();
  render();
  fit();
}
const nameOf = (id: string) =>
  doc.entities.find((e) => e.id === id)?.name || id;
function renderBars() {
  const f = view.focus;
  const bar = $("focus-bar");
  bar.hidden = !f;
  if (f) {
    const names = f.entities.map(nameOf);
    const what =
      names.length > 2
        ? `${names.slice(0, 2).join(", ")} +${names.length - 2}`
        : names.join(", ");
    const dir = f.direction || "both";
    bar.innerHTML = `<span class="focus-label">${dir === "upstream" ? "What feeds" : dir === "downstream" ? "What is fed by" : "Around"} <b>${esc(what)}</b> · ${current.nodes.filter((n) => !n.container || n.collapsed).length} shown</span><span class="seg" role="group" aria-label="Direction">${(["both", "upstream", "downstream"] as const).map((d) => `<button data-dir="${d}" class="${d === dir ? "on" : ""}">${{ both: "↔ Both", upstream: "↑ Feeds", downstream: "↓ Fed" }[d]}</button>`).join("")}</span><span class="seg" role="group" aria-label="Hops">${[1, 2, 3, 0].map((h) => `<button data-hops="${h}" class="${(f.hops ?? 0) === h ? "on" : ""}">${h || "All"}</button>`).join("")}</span><button id="exit-focus">✕ Show everything</button>`;
    bar
      .querySelectorAll<HTMLButtonElement>("[data-dir]")
      .forEach(
        (b) =>
          (b.onclick = () =>
            setFocus({ ...f, direction: b.dataset.dir as Direction })),
      );
    bar
      .querySelectorAll<HTMLButtonElement>("[data-hops]")
      .forEach(
        (b) =>
          (b.onclick = () =>
            setFocus({ ...f, hops: Number(b.dataset.hops) || undefined })),
      );
    $("exit-focus").onclick = () => setFocus(null);
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
      setFocus({ entities: [...picked], hops: 0, direction: "both" });
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
    content += `<g class="node space ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="space-body" width="${n.width}" height="${n.height}" rx="12" fill="${fills[Math.min(n.depth, 2)]}"/><g class="space-header"><path d="M12 0 H${n.width - 12} Q${n.width} 0 ${n.width} 12 V52 H0 V12 Q0 0 12 0 Z" fill="${headers[Math.min(n.depth, 2)]}"/><path class="space-divider" d="M0 52 H${n.width}"/><text class="glyph" x="15" y="35">${icon(e.kind)}</text><text class="kind" x="43" y="18">${esc(e.kind.toUpperCase())}</text><text class="name" x="43" y="36">${esc(e.name)}</text><text class="space-count" x="${n.width - 50}" y="32" text-anchor="end">${n.children} inside</text>${n.children ? `<g class="fold" data-fold="${esc(n.id)}"><rect x="${n.width - 34}" y="16" width="20" height="20" rx="5"/><text x="${n.width - 29}" y="30">−</text></g>` : ""}</g></g>`;
  }
  for (const b of current.bundles) {
    const layer = doc.layers.find((l) => l.id === b.layer)!;
    for (const branch of b.branches) {
      if (!branch.points.length) continue;
      const path = branch.points
        .map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`)
        .join(" ");
      content += `<path class="route" stroke="${esc(layer.color)}" d="${path}" ${branch.role === "target" ? `marker-start="url(#arrow-${esc(layer.id)})"` : ""}><title>${esc(b.connections.map((id) => doc.connections.find((c) => c.id === id)!.name).join(", "))}</title></path>`;
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
    content += `<g class="node ${n.collapsed ? "collapsed-space" : ""} ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="card" width="${n.width}" height="${n.height}" rx="9"/><text class="glyph" x="15" y="42">${icon(e.kind)}</text><text class="kind" x="45" y="23">${esc(e.kind.toUpperCase())}</text><text class="name" x="45" y="44">${esc(e.name.length > 19 ? e.name.slice(0, 18) + "…" : e.name)}</text>${n.children ? `<g class="fold" data-fold="${esc(e.id)}"><rect x="173" y="10" width="19" height="19" rx="4" fill="#e2edd8"/><text x="178" y="24">+</text></g><text class="kind" x="16" y="62">${n.children} hidden inside</text>` : ""}</g>`;
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
            ? { pitch: 10, first: 9, cls: " tiny" }
            : null;
    if (!fit) {
      const small = b.height < 14;
      return `<text class="slot-pos${small ? " tiny" : ""}" x="${x}" y="${b.y + (small ? 9 : 13)}"${anchor}>${esc(b.position || "·")}</text>`;
    }
    return parts
      .map(
        (t, k) =>
          `<text class="slot-pos${fit.cls}" x="${x}" y="${b.y + fit.first + k * fit.pitch}"${anchor}>${esc(t)}</text>`,
      )
      .join("");
  };
  const slot = (b: PortBox, cls: string, title: string) => {
    const p = b.id ? portOf(b.id) : undefined;
    const text = [b.label, ...(b.id ? fed(b.id) : [])].filter(Boolean)[0] || "";
    const right = panel && b.side === "right";
    // Past the handle (and badge) on the right column; past the number on
    // the left.
    const handleW = panel && b.id ? HANDLE + 4 : 0;
    const badgeW = p?.protection ? 19 : 0;
    const labelX = right ? b.x + handleW + badgeW + 5 : b.x + 38;
    const posX = right ? b.x + b.width - 5 : b.x + 5;
    const max = Math.max(
      4,
      Math.floor((b.width - 44 - handleW - badgeW) / 6.2),
    );
    const small = b.height < 14;
    return `<g class="slot ${cls}" ${b.id ? `data-port="${esc(b.id)}"` : ""}><rect x="${b.x}" y="${b.y}" width="${b.width}" height="${b.height}" rx="3"/>${positions(b, posX, right)}<text class="slot-label ${small ? "tiny" : ""}" x="${labelX}" y="${b.y + (small ? 9 : 13)}">${esc(text.length > max ? text.slice(0, max - 1) + "…" : text)}</text>${handle(b, p)}${badge(b, p)}<title>${esc(title)}</title></g>`;
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
      body += `<g class="slot used tied"><rect x="${b.x}" y="${b.y}" width="${b.width}" height="${b.height}" rx="3"/>${positions(b, right ? b.x + b.width - 5 : b.x + 5, right)}<text class="slot-label tiny" x="${labelX}" y="${b.y + 9}">↑ ${esc(b.id)}</text>${handle(b, portOf(b.id))}</g>`;
      // Drawn last, over any breaker the tie crosses (a quad's inner pair).
      ties += `<path class="tie" d="M${x},${first.y + first.height / 2} V${b.y + b.height / 2}"><title>${esc(b.id)}: one linked breaker</title></path>`;
      continue;
    }
    const names = fed(b.id);
    const r = rating(portOf(b.id));
    body += slot(
      b,
      names.length ? "used" : "spare",
      `${b.position ? b.position + " · " : ""}${b.id}${r ? " · " + r : ""}${b.label ? " · " + b.label : ""}${names.length ? "\n→ " + names.join(", ") : "\nNothing connected"}`,
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
  return `<g class="node faceplate ${selected === n.id ? "selected" : ""} ${picked.has(n.id) ? "picked" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="card" width="${n.width}" height="${n.height}" rx="9"/><text class="glyph" x="15" y="34">${icon(e.kind)}</text><text class="kind" x="45" y="20">${esc(e.kind.toUpperCase())} · ${kindLabel}</text><text class="name" x="45" y="39">${esc(e.name.length > 34 ? e.name.slice(0, 33) + "…" : e.name)}</text>${body}</g>`;
}
function renderTree() {
  const query = $<HTMLInputElement>("search").value.toLowerCase();
  let html = "";
  function walk(parent: string | null, depth: number) {
    for (const e of doc.entities.filter((e) => e.parent === parent)) {
      const children = doc.entities.some((n) => n.parent === e.id);
      if (!query || e.name.toLowerCase().includes(query))
        html += `<div class="tree-row ${selected === e.id ? "selected" : ""}" data-select="${esc(e.id)}" style="padding-left:${depth * 13 + 3}px"><button data-fold="${esc(e.id)}" ${children ? "" : "disabled"} aria-label="Toggle ${esc(e.name)}">${children ? (view.collapsed.includes(e.id) ? "▸" : "▾") : "·"}</button><span class="kind-icon">${icon(e.kind)}</span><span>${esc(e.name)}</span></div>`;
      if (!view.collapsed.includes(e.id) || query) walk(e.id, depth + 1);
    }
  }
  walk(null, 0);
  $("tree").innerHTML = html;
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
const portOrder = (a: Port, b: Port) =>
  Number(!!b.main) - Number(!!a.main) ||
  (a.position ?? 1e9) - (b.position ?? 1e9) ||
  (a.halves?.[0] || a.half || "").localeCompare(
    b.halves?.[0] || b.half || "",
  ) ||
  a.id.localeCompare(b.id, undefined, { numeric: true });
function spaces(e: Entity, p: Port) {
  if (p.main) return "Main";
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
        `<tr><td>${esc(r.position)}</td><td><b>${esc(r.port.id)}</b>${ratingHtml(r.port)}${r.port.label ? `<small>${esc(r.port.label)}</small>` : ""}</td><td>${cell(r.connections)}</td></tr>`,
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
      return `<div class="conn"><span class="swatch" style="background:${esc(layer.color)}"></span><div><div>${arrow} ${others}</div><small>${esc(c.name)}</small></div>${select}</div>`;
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
        `<div class="port-row" data-row="${i}" ${p.orig ? `data-orig="${esc(p.orig)}"` : ""}><div class="l1"><input name="id" value="${esc(p.id)}" required maxlength="40" aria-label="Port ID"><input name="position" inputmode="numeric" pattern="[0-9]{1,3}|[Mm][Aa][Ii][Nn]" placeholder="#" title="Starting space or port number, or main for the supply" aria-label="Position" value="${p.main ? "main" : (p.position ?? "")}"><select name="poles" aria-label="Poles">${[1, 2, 3].map((n) => `<option ${(p.poles || 1) === n ? "selected" : ""}>${n}</option>`).join("")}</select><input name="half" pattern="[ABab]|([ABab-]/)+[ABab-]" title="A or B for a tandem half; one per space for a linked breaker, e.g. B/A (- for a whole space)" aria-label="Half" value="${esc(p.halves?.length ? p.halves.map((h) => h || "-").join("/") : p.half || "")}"><button type="button" class="remove-port" aria-label="Remove port ${esc(p.id)}">✕</button></div><div class="l2"><input name="amps" inputmode="numeric" pattern="[0-9]{1,4}" placeholder="A" title="Rating in amps" aria-label="Amps" value="${p.amps ?? ""}"><select name="protection" aria-label="Protection" title="Built-in protection"><option value="">—</option>${(["gfci", "afci", "dual"] as const).map((k) => `<option value="${k}" ${p.protection === k ? "selected" : ""}>${PROTECTION_NAME[k]}</option>`).join("")}</select><input name="label" value="${esc(p.label || "")}" maxlength="200" placeholder="Label" aria-label="Label"></div></div>`,
    )
    .join(
      "",
    )}</div><div class="button-row"><button type="button" id="add-port">＋ Port</button><button type="button" id="number-ports" title="Add numbered ports up to a count">＋ Numbered…</button><button>Apply ports</button></div></form></details>`;
}
let portEditorOpen = false;
function renderInspector() {
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
    `<span class="eyebrow">SELECTED ${esc(e.kind.toUpperCase())}</span><h2>${esc(e.name)}</h2><p class="muted">${doc.connections.filter((c) => c.endpoints.some((ep) => ep.entity === e.id)).length} connections · ${doc.entities.filter((n) => n.parent === e.id).length} children</p>${e.links.map((l) => `<div class="external-link"><b>${esc(l.provider.replaceAll("_", " ").toUpperCase())} · ${esc(l.kind)}</b>${esc(l.external_id)}</div>`).join("")}${actionsHtml(e)}${scheduleHtml(e)}${connectionsHtml(e, writable)}${
      writable
        ? `<form id="entity-form"><label class="field">Name<input name="name" value="${esc(e.name)}" required maxlength="150"></label><label class="field">Type<input name="kind" list="kinds" value="${esc(e.kind)}" required maxlength="40"><datalist id="kinds">${kinds.map((k) => `<option value="${esc(k)}">`).join("")}</datalist></label><label class="field">Parent<select name="parent"><option value="">Top level</option>${doc.entities
            .filter((n) => !descendants.has(n.id))
            .map(
              (n) =>
                `<option value="${esc(n.id)}" ${e.parent === n.id ? "selected" : ""}>${esc(n.name)}</option>`,
            )
            .join(
              "",
            )}</select></label><div class="button-row"><button>Apply</button><button type="button" id="delete-entity" class="danger">Delete${descendants.size > 1 ? ` (${descendants.size})` : ""}</button></div></form>${leaf ? portEditorHtml(e) : ""}<form id="connect-form" class="connect-form"><h3>Connect this thing</h3><label class="field">Connection name<input name="name" required placeholder="e.g. Office power"></label><label class="field">Layer<select name="layer">${doc.layers.map((l) => `<option value="${esc(l.id)}">${esc(l.name)}</option>`).join("")}</select></label><label class="field">Other endpoints<select name="targets" multiple required size="4">${doc.entities
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
          if (/^main$/i.test(v("position"))) p.main = true;
          else if (v("position")) p.position = Number(v("position"));
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
        setFocus({
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
}
// Phones have no inspector column: tapping a thing opens this sheet instead.
let sheetId: string | null = null;
const narrow = () => matchMedia("(max-width: 800px)").matches;
function renderSheet() {
  const sheet = $("sheet");
  const e = sheetId ? doc.entities.find((x) => x.id === sheetId) : undefined;
  sheet.hidden = !e;
  if (!e) return;
  const inside = doc.entities.filter((n) => n.parent === e.id).length;
  const links = doc.connections.filter((c) =>
    c.endpoints.some((ep) => ep.entity === e.id),
  ).length;
  sheet.innerHTML = `<div class="sheet-head"><div><span class="eyebrow">${esc(e.kind.toUpperCase())}</span><h2>${esc(e.name)}</h2><p class="muted">${links} connections${inside ? ` · ${inside} inside` : ""}</p></div><button class="sheet-close" aria-label="Close">✕</button></div>${actionsHtml(e)}${scheduleHtml(e)}${connectionsHtml(e, false)}`;
  sheet.querySelector<HTMLButtonElement>(".sheet-close")!.onclick = () => {
    sheetId = null;
    renderSheet();
  };
  wireEntityActions(sheet, e);
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
  if (!view.focus) delete view.focus;
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
  if (tap && !pointers.size && ev.type === "pointerup" && narrow()) {
    sheetId = tap.id;
    renderSheet();
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
  tap = id ? { id, at: pointerPoint(ev) } : null;
  if (id) {
    selected = id;
    renderTree();
    renderInspector();
    document
      .querySelectorAll(".node.selected")
      .forEach((n) => n.classList.remove("selected"));
    target.closest(".node")?.classList.add("selected");
    const node = current.nodes.find((n) => n.id === id)!;
    if (node.container && !node.collapsed && !target.closest(".space-header"))
      id = null;
    // A focus is laid out compactly for reading; arrange the full view.
    if (view.focus && id) {
      id = null;
      status("Exit the focus to rearrange things");
    }
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
let tap: { id: string; at: Point } | null = null;
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
    selected = row.getAttribute("data-select")!;
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
$("search").oninput = renderTree;
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
  render();
  status(
    offline
      ? "Offline viewer · Rust/WASM engine"
      : `Rust/WASM engine · ${writable ? "SQLite connected" : "Read-only share"}`,
  );
}
run(start)();
