// Full routing off the main thread. The page posts the compiled WASM module
// once, then {generation, input} layout requests; each reply carries the
// routed layout as a JSON string, tagged with the request's generation.
import init, { compute } from "./pkg/stufftracker_core";

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
  try {
    await ready;
    reply({ generation: m.generation, layout: compute(m.input) });
  } catch (e) {
    reply({ generation: m.generation, error: String(e) });
  }
};
