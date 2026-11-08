import init, { compute, move_node } from "./pkg/stufftracker_core";
declare const WASM_BASE64: string;
declare const WORKER_SOURCE: string;
type Point = { x: number; y: number };
type Entity = {
  id: string;
  name: string;
  kind: string;
  parent: string | null;
  links: { provider: string; kind: string; external_id: string }[];
};
type Role = "source" | "target" | "peer";
type Connection = {
  id: string;
  name: string;
  layer: string;
  endpoints: { entity: string; role: Role }[];
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
function render() {
  computeLayout();
  renderScene();
  renderTree();
  renderLayers();
  renderInspector();

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
    content += `<g class="node space ${selected === n.id ? "selected" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="space-body" width="${n.width}" height="${n.height}" rx="12" fill="${fills[Math.min(n.depth, 2)]}"/><g class="space-header"><path d="M12 0 H${n.width - 12} Q${n.width} 0 ${n.width} 12 V52 H0 V12 Q0 0 12 0 Z" fill="${headers[Math.min(n.depth, 2)]}"/><path class="space-divider" d="M0 52 H${n.width}"/><text class="glyph" x="15" y="35">${icon(e.kind)}</text><text class="kind" x="43" y="18">${esc(e.kind.toUpperCase())}</text><text class="name" x="43" y="36">${esc(e.name)}</text><text class="space-count" x="${n.width - 50}" y="32" text-anchor="end">${n.children} inside</text>${n.children ? `<g class="fold" data-fold="${esc(n.id)}"><rect x="${n.width - 34}" y="16" width="20" height="20" rx="5"/><text x="${n.width - 29}" y="30">−</text></g>` : ""}</g></g>`;
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
    content += `<g class="node ${n.collapsed ? "collapsed-space" : ""} ${selected === n.id ? "selected" : ""}" data-id="${esc(n.id)}" data-parent="${esc(e.parent || "")}" data-width="${n.width}" data-height="${n.height}" transform="translate(${n.position.x},${n.position.y})"><rect class="card" width="${n.width}" height="${n.height}" rx="9"/><text class="glyph" x="15" y="42">${icon(e.kind)}</text><text class="kind" x="45" y="23">${esc(e.kind.toUpperCase())}</text><text class="name" x="45" y="44">${esc(e.name.length > 19 ? e.name.slice(0, 18) + "…" : e.name)}</text>${n.children ? `<g class="fold" data-fold="${esc(e.id)}"><rect x="173" y="10" width="19" height="19" rx="4" fill="#e2edd8"/><text x="178" y="24">+</text></g><text class="kind" x="16" y="62">${n.children} hidden inside</text>` : ""}</g>`;
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
  $("inspector").innerHTML =
    `<span class="eyebrow">SELECTED ${esc(e.kind.toUpperCase())}</span><h2>${esc(e.name)}</h2><p class="muted">${doc.connections.filter((c) => c.endpoints.some((ep) => ep.entity === e.id)).length} connections · ${doc.entities.filter((n) => n.parent === e.id).length} children</p>${e.links.map((l) => `<div class="external-link"><b>${esc(l.provider.replaceAll("_", " ").toUpperCase())} · ${esc(l.kind)}</b>${esc(l.external_id)}</div>`).join("")}${
      writable
        ? `<form id="entity-form"><label class="field">Name<input name="name" value="${esc(e.name)}" required maxlength="150"></label><label class="field">Type<select name="kind">${["space", "device", "equipment", "service"].map((k) => `<option ${k === e.kind ? "selected" : ""}>${k}</option>`).join("")}</select></label><label class="field">Parent<select name="parent"><option value="">Top level</option>${doc.entities
            .filter((n) => !descendants.has(n.id))
            .map(
              (n) =>
                `<option value="${esc(n.id)}" ${e.parent === n.id ? "selected" : ""}>${esc(n.name)}</option>`,
            )
            .join(
              "",
            )}</select></label><div class="button-row"><button>Apply</button><button type="button" id="delete-entity" class="danger">Delete${descendants.size > 1 ? ` (${descendants.size})` : ""}</button></div></form><form id="connect-form" class="connect-form"><h3>Connect this thing</h3><label class="field">Connection name<input name="name" required placeholder="e.g. Office power"></label><label class="field">Layer<select name="layer">${doc.layers.map((l) => `<option value="${esc(l.id)}">${esc(l.name)}</option>`).join("")}</select></label><label class="field">Other endpoints<select name="targets" multiple required size="4">${doc.entities
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
      e.kind = String(f.get("kind"));
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
  camera.scale = Math.min(
    1.2,
    (box.width - 30) / (maxX - minX),
    (box.height - 40) / (maxY - minY),
  );
  camera.x =
    (box.width - (maxX - minX) * camera.scale) / 2 - minX * camera.scale;
  camera.y = 20 - minY * camera.scale;
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
// Pointer events arrive far faster than a move + relayout completes on a
// real inventory. Only the latest target position matters: `latestMove` is
// overwritten on every pointermove and `moveSeq` counts them, while
// `appliedSeq` is the newest move the view reflects. Layout runs in a worker
// when one is available (one request in flight, always for the newest
// position) so the page keeps panning and painting; otherwise it runs on this
// thread at most once per animation frame.
let latestMove: { id: string; position: Point } | null = null;
let moveSeq = 0,
  appliedSeq = 0,
  moveFrame = 0;
let layoutWorker: Worker | null = null,
  workerBusy = false,
  // Bumped whenever this thread applies a move itself; any worker reply
  // computed against an older view is then discarded.
  workerGeneration = 0;
// `data-layout-pending` marks the gap between a drag position and the layout
// that reflects it, so automation can wait for the settled scene.
function markSettled() {
  if (appliedSeq === moveSeq) delete svg.dataset.layoutPending;
}
function requestMove(move: { id: string; position: Point }) {
  latestMove = move;
  moveSeq++;
  svg.dataset.layoutPending = "";
  if (layoutWorker) {
    if (!workerBusy) postMove();
  } else if (!moveFrame) moveFrame = requestAnimationFrame(settleDrag);
}
function postMove() {
  if (!layoutWorker || !latestMove || appliedSeq === moveSeq) return;
  workerBusy = true;
  layoutWorker.postMessage({
    generation: workerGeneration,
    seq: moveSeq,
    document: JSON.stringify(doc),
    view: JSON.stringify(view),
    entity: latestMove.id,
    position: latestMove.position,
  });
}
// Bring the view up to the newest move synchronously. Runs per frame without
// a worker, and with one whenever something else is about to touch the view
// before the worker has delivered the final position.
function settleDrag() {
  if (moveFrame) cancelAnimationFrame(moveFrame);
  moveFrame = 0;
  if (!latestMove || appliedSeq === moveSeq) return;
  workerGeneration++;
  view = JSON.parse(
    move_node(
      JSON.stringify({
        document: doc,
        view,
        entity: latestMove.id,
        position: latestMove.position,
      }),
    ),
  );
  appliedSeq = moveSeq;
  markSettled();
  computeLayout();
  renderScene();
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
    return; // e.g. a sandboxed export; stay on the per-frame path
  }
  const fallBack = () => {
    worker.terminate();
    layoutWorker = null;
    workerBusy = false;
    settleDrag();
  };
  worker.onerror = fallBack;
  worker.onmessage = (ev) => {
    workerBusy = false;
    const r = ev.data;
    if (r.error) {
      status(`Layout worker failed, laying out in page: ${r.error}`, true);
      fallBack();
      return;
    }
    if (r.generation === workerGeneration && r.seq > appliedSeq) {
      view = JSON.parse(r.view);
      appliedSeq = r.seq;
      markSettled();
      applyLayout(JSON.parse(r.layout));
      renderScene();
    }
    postMove();
  };
  layoutWorker = worker;
}
// Any new interaction first lands an in-flight drag, so handlers always see
// (and build on) the view the user last saw the node dropped into.
document.addEventListener("pointerdown", () => settleDrag(), true);
document.addEventListener("keydown", () => settleDrag(), true);
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
  const { center, distance } = pinchGeometry();
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
function endPointer(ev: PointerEvent) {
  if (!pointers.delete(ev.pointerId)) return;
  // Land the drag where the pointer let go (the worker delivers it itself).
  if (!layoutWorker) settleDrag();
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
  }
  drag = {
    id,
    start: { x: ev.clientX - r.left, y: ev.clientY - r.top },
    origin: id
      ? { ...current.nodes.find((n) => n.id === id)!.position }
      : { x: camera.x, y: camera.y },
  };
  svg.setPointerCapture(ev.pointerId);
});
svg.addEventListener("pointermove", (ev) => {
  if (!pointers.has(ev.pointerId)) return;
  pointers.set(ev.pointerId, pointerPoint(ev));
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
