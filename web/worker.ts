// Drag layout off the main thread. The page posts the compiled WASM module
// once, then one {document, view, entity, position} request at a time; each
// reply carries the moved view and its full layout as JSON strings.
import init, { compute, move_node } from "./pkg/stufftracker_core";

// Typed against the DOM lib, where `self` is a Window.
const reply = (msg: object) =>
  (self as unknown as { postMessage(msg: object): void }).postMessage(msg);
let ready: Promise<unknown> | null = null;
self.onmessage = async (ev: MessageEvent) => {
  const m = ev.data;
  if (m.module) {
    ready = init({ module_or_path: m.module });
    return;
  }
  const tag = { generation: m.generation, seq: m.seq };
  try {
    await ready;
    const view = move_node(
      `{"document":${m.document},"view":${m.view},"entity":${JSON.stringify(m.entity)},"position":${JSON.stringify(m.position)}}`,
    );
    const layout = compute(`{"document":${m.document},"view":${view}}`);
    reply({ ...tag, view, layout });
  } catch (e) {
    reply({ ...tag, error: String(e) });
  }
};
