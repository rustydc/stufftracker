import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { request } from "node:http";
import { mkdtempSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
mkdirSync("work", { recursive: true });
const root = mkdtempSync(resolve("work/grain-test-"));
const socket = root + "/http.sock";
function http(path, method = "GET", body, permissions) {
  return new Promise((done, reject) => {
    const req = request(
      {
        socketPath: socket,
        path,
        method,
        headers: {
          "Content-Type": "application/json",
          ...(permissions ? { "X-Darude-Permissions": permissions } : {}),
        },
      },
      (res) => {
        let data = "";
        res.on("data", (c) => (data += c));
        res.on("end", () => done({ status: res.statusCode, body: data }));
      },
    );
    req.on("error", reject);
    if (body) req.write(JSON.stringify(body));
    req.end();
  });
}
async function boot() {
  const child = spawn(resolve("target/release/stufftracker"), {
    env: {
      ...process.env,
      DARUDE_STATE_DIR: root + "/state",
      DARUDE_HTTP_SOCKET: socket,
    },
    stdio: ["ignore", "ignore", "pipe"],
  });
  await new Promise((done, reject) => {
    const timeout = setTimeout(
      () => reject(new Error("Grain did not reach readiness")),
      10000,
    );
    child.stderr.on("data", (b) => {
      if (b.toString().includes("HTTP bound on")) {
        clearTimeout(timeout);
        done();
      }
    });
    child.on("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`Grain exited ${code}`));
    });
  });
  return child;
}
async function stop(child) {
  const exit = new Promise((done) => child.once("exit", done));
  child.kill("SIGTERM");
  await exit;
}
let child = await boot();
try {
  assert.equal((await http("/")).status, 200);
  const snapshot = JSON.parse((await http("/api/document")).body);
  assert.equal(snapshot.document.entities.length, 17);
  assert.equal((await http("/api/document", "PUT", snapshot)).status, 403);
  assert.equal(
    (await http("/api/document", "PUT", snapshot, "read")).status,
    403,
  );
  snapshot.document.entities[0].name = "Grain persistence check";
  assert.equal(
    (await http("/api/document", "PUT", snapshot, "read,write")).status,
    200,
  );
  assert.equal(
    (await http("/api/document", "PUT", snapshot, "owner")).status,
    409,
  );
  assert.equal(
    (
      await http(
        "/api/views",
        "POST",
        {
          id: "test",
          name: "Grain view",
          view: {
            positions: {},
            collapsed: ["kitchen"],
            hidden_layers: ["water"],
          },
        },
        "write",
      )
    ).status,
    204,
  );
  await stop(child);
  child = await boot();
  const restored = JSON.parse((await http("/api/document")).body);
  assert.equal(restored.document.entities[0].name, "Grain persistence check");
  assert.equal(restored.revision, 2);
  assert.equal(
    JSON.parse((await http("/api/views")).body)[0].name,
    "Grain view",
  );
  assert.equal(
    (await http("/api/views/test", "DELETE", null, "read")).status,
    403,
  );
  assert.equal(
    (await http("/api/views/test", "DELETE", null, "owner")).status,
    204,
  );
  console.log(
    "Grain contract passed: cold Unix-socket boot, fail-closed writes, read/write shares, conflict checks, saved views, restart persistence.",
  );
} finally {
  await stop(child);
}
