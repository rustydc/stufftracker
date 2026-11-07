import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { build } from "esbuild";
execFileSync(
  "cargo",
  [
    "build",
    "-p",
    "stufftracker-core",
    "--release",
    "--target",
    "wasm32-unknown-unknown",
  ],
  { stdio: "inherit" },
);
execFileSync(
  "wasm-bindgen",
  [
    "target/wasm32-unknown-unknown/release/stufftracker_core.wasm",
    "--out-dir",
    "web/pkg",
    "--target",
    "web",
  ],
  { stdio: "inherit" },
);
const wasm = readFileSync("web/pkg/stufftracker_core_bg.wasm").toString(
  "base64",
);
const result = await build({
  entryPoints: ["web/main.ts"],
  bundle: true,
  write: false,
  format: "iife",
  target: "es2022",
  minify: true,
  logOverride: { "empty-import-meta": "silent" },
  define: { WASM_BASE64: JSON.stringify(wasm) },
});
const script = result.outputFiles[0].text.replaceAll("</script", "<\\/script");
const html = readFileSync("web/index.html", "utf8")
  .replace("/*STYLES*/", readFileSync("web/style.css", "utf8"))
  .replace("/*SCRIPT*/", () => script);
mkdirSync("dist", { recursive: true });
writeFileSync("dist/index.html", html);
console.log(
  `Built self-contained viewer (${Math.round(Buffer.byteLength(html) / 1024)} KiB, including Rust/WASM).`,
);
