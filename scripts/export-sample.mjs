import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
const html = readFileSync("dist/index.html", "utf8");
const document = JSON.parse(readFileSync("sample.json", "utf8"));
const snapshot = JSON.stringify({
  document,
  view: { layout_version: 2, positions: {}, collapsed: [], hidden_layers: [] },
}).replaceAll("<", "\\u003c");
const marker =
  /<script id="snapshot" type="application\/json">\s*null\s*<\/script>/;
if (!marker.test(html))
  throw new Error("The built viewer snapshot placeholder is missing");
mkdirSync("work", { recursive: true });
writeFileSync(
  "work/sample-viewer.html",
  html.replace(
    marker,
    () => `<script id="snapshot" type="application/json">${snapshot}</script>`,
  ),
);
console.log(
  "Exported work/sample-viewer.html with the sample nested hierarchy and embedded Rust/WASM.",
);
