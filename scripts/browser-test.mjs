import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync } from "node:fs";
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_PATH
    ? { executablePath: process.env.CHROMIUM_PATH }
    : {}),
});
const page = await browser.newPage({
  viewport: { width: 1500, height: 1000 },
  acceptDownloads: true,
});
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
mkdirSync("work", { recursive: true });
try {
  for (const v of await (await page.request.get(`${base}api/views`)).json())
    if (v.name === "Browser test perspective")
      await page.request.delete(`${base}api/views/${v.id}`);
  await page.goto(base);
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  assert.equal(await page.locator(".node").count(), 17);
  const transforms = async () =>
    page
      .locator(".node")
      .evaluateAll((ns) =>
        Object.fromEntries(
          ns.map((n) => [n.dataset.id, n.getAttribute("transform")]),
        ),
      );
  const before = await transforms();
  await page.locator('.tree-row [data-fold="kitchen"]').click();
  assert.equal(await page.locator('.node[data-id="lamp"]').count(), 0);
  const collapsed = await transforms();
  for (const [id, pos] of Object.entries(collapsed))
    assert.equal(pos, before[id]);
  await page.locator('.tree-row [data-fold="kitchen"]').click();
  assert.deepEqual(await transforms(), before);
  await page
    .getByRole("checkbox", { name: "Power layer", exact: true })
    .uncheck();
  assert.deepEqual(await transforms(), before);
  await page
    .getByRole("checkbox", { name: "Power layer", exact: true })
    .check();
  const node = page.locator('.node[data-id="lamp"]');
  const box = await node.boundingBox();
  await page.mouse.move(box.x + 35, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 65, box.y + 45, { steps: 4 });
  await page.mouse.up();
  const moved = await transforms();
  assert.notEqual(moved.lamp, before.lamp);
  for (const id of Object.keys(before))
    if (id !== "lamp") assert.equal(moved[id], before[id]);
  await page.getByRole("button", { name: "Save view", exact: true }).click();
  await page.locator("#view-name").fill("Browser test perspective");
  await page.locator("#view-form button.primary").click();
  await page.waitForFunction(
    () =>
      document.querySelector("#status").textContent === "View saved to SQLite",
  );
  await page.reload();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  await page
    .locator("#views")
    .selectOption({ label: "Browser test perspective" });
  assert.deepEqual(await transforms(), moved);
  // Native layout and browser WASM use the same endpoint/role projection.
  const snapshot = await (await page.request.get(`${base}api/document`)).json();
  const native = await (
    await page.request.post(`${base}api/layout`, {
      data: {
        document: snapshot.document,
        view: { positions: {}, collapsed: [], hidden_layers: [] },
      },
    })
  ).json();
  assert.equal(native.nodes.length, 17);
  assert(
    native.bundles.some(
      (b) => b.branches.filter((p) => p.role === "target").length > 1,
    ),
  );
  assert((await page.locator("path[marker-start]").count()) > 0);
  // Add and save an entity through the UI; stale writes must be rejected.
  await page.locator("#add").click();
  await page
    .locator('#entity-form input[name="name"]')
    .fill("Browser test device");
  await page
    .locator("#entity-form button")
    .filter({ hasText: "Apply" })
    .click();
  await page.locator("#save-document").click();
  await page.waitForFunction(
    () =>
      document.querySelector("#status").textContent ===
      "Inventory saved to SQLite",
  );
  const stale = await page.request.put(`${base}api/document`, {
    data: snapshot,
  });
  assert.equal(stale.status(), 409);
  await page.reload();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  assert(
    await page
      .locator("#tree")
      .textContent()
      .then((t) => t.includes("Browser test device")),
  );
  // Clean up test entity, leaving the sample inventory for users.
  await page
    .locator(".tree-row")
    .filter({ hasText: "Browser test device" })
    .click();
  await page.locator("#delete-entity").click();
  await page.locator("#save-document").click();
  await page.waitForFunction(
    () =>
      document.querySelector("#status").textContent ===
      "Inventory saved to SQLite",
  );
  await page.locator('.tree-row[data-select="kitchen"]').click();
  await page.locator("#fit").click();
  const downloadPromise = page.waitForEvent("download");
  await page.locator("#export").click();
  const download = await downloadPromise;
  const path = process.cwd() + "/work/exported-view.html";
  await download.saveAs(path);
  assert(readFileSync(path, "utf8").includes("application/json"));
  const context = await browser.newContext({
    offline: true,
    viewport: { width: 1500, height: 1000 },
  });
  const exported = await context.newPage();
  const requests = [];
  exported.on("request", (r) => {
    if (/^https?:/.test(r.url())) requests.push(r.url());
  });
  exported.on("pageerror", (e) => errors.push(e.message));
  await exported.setContent(readFileSync(path, "utf8"), { waitUntil: "load" });
  await exported.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("Offline viewer"),
  );
  assert.equal(await exported.locator(".node").count(), 17);
  await exported.locator('.tree-row [data-fold="kitchen"]').click();
  assert.equal(await exported.locator('.node[data-id="lamp"]').count(), 0);
  await exported
    .getByRole("checkbox", { name: "Power layer", exact: true })
    .uncheck();
  assert.equal(await exported.locator("#save-document").isVisible(), false);
  assert.deepEqual(requests, []);
  await exported.screenshot({
    path: "work/offline-viewer.png",
    fullPage: true,
  });
  await context.close();
  await page.screenshot({ path: "work/app.png", fullPage: true });
  for (const v of await (await page.request.get(`${base}api/views`)).json()) {
    if (v.name === "Browser test perspective")
      await page.request.delete(`${base}api/views/${v.id}`);
  }
  assert.deepEqual(errors, []);
  console.log(
    "Browser checks passed: WASM, drag stability, collapse/layers, SQLite editing, saved views, revision conflicts, offline HTML export.",
  );
} finally {
  await browser.close();
}
