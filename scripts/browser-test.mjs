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
// Drag layout may finish in a worker after pointerup; wait for it to land.
const settled = (p) =>
  p.waitForFunction(
    () => !("layoutPending" in document.querySelector("#graph").dataset),
  );
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
mkdirSync("work", { recursive: true });
try {
  for (const v of await (await page.request.get(`${base}api/views`)).json())
    if (v.name === "Browser test perspective")
      await page.request.delete(`${base}api/views/${v.id}`);
  // The page is always revalidated, so a new build shows on the next load.
  const first = await page.request.get(base);
  assert.equal(first.headers()["cache-control"], "no-cache");
  const etag = first.headers()["etag"];
  assert.ok(etag, "the viewer has an ETag");
  const again = await page.request.get(base, {
    headers: { "If-None-Match": etag },
  });
  assert.equal(again.status(), 304);
  await page.goto(base + "#everything");
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
  const geometry = async () =>
    page.locator(".node").evaluateAll((ns) =>
      Object.fromEntries(
        ns.map((n) => {
          const [x, y] = n
            .getAttribute("transform")
            .match(/-?[\d.]+/g)
            .map(Number);
          return [
            n.dataset.id,
            {
              x,
              y,
              width: Number(n.dataset.width),
              height: Number(n.dataset.height),
              parent: n.dataset.parent,
            },
          ];
        }),
      ),
    );
  // Siblings never overlap: drags push or swap them instead.
  const apart = (g) => {
    for (const [a, p] of Object.entries(g))
      for (const [b, q] of Object.entries(g))
        if (a < b && p.parent === q.parent)
          assert(
            p.x + p.width <= q.x ||
              q.x + q.width <= p.x ||
              p.y + p.height <= q.y ||
              q.y + q.height <= p.y,
            `${a} overlaps ${b}`,
          );
  };
  const nested = await geometry();
  assert.equal(await page.locator(".space").count(), 7);
  for (const n of Object.values(nested))
    if (n.parent) {
      const p = nested[n.parent];
      assert(
        n.x >= p.x &&
          n.y >= p.y + 52 &&
          n.x + n.width <= p.x + p.width &&
          n.y + n.height <= p.y + p.height,
      );
    }
  const kitchenHeader = await page
    .locator('.space[data-id="kitchen"] .space-header')
    .boundingBox();
  await page.mouse.move(kitchenHeader.x + 80, kitchenHeader.y + 20);
  await page.mouse.down();
  await page.mouse.move(kitchenHeader.x + 88, kitchenHeader.y + 28, {
    steps: 3,
  });
  await page.mouse.up();
  await settled(page);
  assert.equal(
    await page.evaluate(() => window.getSelection()?.toString()),
    "",
  );
  assert.equal(
    await page
      .locator("#graph")
      .evaluate((el) => getComputedStyle(el).userSelect),
    "none",
  );
  const grouped = await geometry();
  const dx = grouped.kitchen.x - nested.kitchen.x,
    dy = grouped.kitchen.y - nested.kitchen.y;
  assert(dx > 0 && dy > 0);
  // The kitchen's subtree moves rigidly; neighbours may be pushed aside, but
  // siblings never overlap.
  for (const id of ["kitchen", "lamp", "sensor", "tap"]) {
    assert(Math.abs(grouped[id].x - nested[id].x - dx) < 0.001);
    assert(Math.abs(grouped[id].y - nested[id].y - dy) < 0.001);
  }
  apart(grouped);
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
  await settled(page);
  let moved = await transforms();
  assert.notEqual(moved.lamp, before.lamp);
  apart(await geometry());
  // Dragging one box onto another swaps them.
  {
    const g = await geometry();
    const sensor = await page.locator('.node[data-id="sensor"]').boundingBox();
    const lamp = await page.locator('.node[data-id="lamp"]').boundingBox();
    await page.mouse.move(sensor.x + 30, sensor.y + 20);
    await page.mouse.down();
    await page.mouse.move(lamp.x + 30, lamp.y + 30, { steps: 6 });
    await page.mouse.up();
    await settled(page);
    const swapped = await geometry();
    assert.equal(swapped.lamp.x, g.sensor.x);
    assert.equal(swapped.lamp.y, g.sensor.y);
    apart(swapped);
    moved = await transforms();
  }
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
  assert.deepEqual(native.unrouted_connections, []);
  const workstation = native.bundles.flatMap((bundle) =>
    bundle.branches.filter((branch) => branch.entity === "desk"),
  );
  assert.equal(workstation.length, 1);
  assert(
    workstation[0].points.length <= 4,
    "Workstation route should not add an elbow at each nested boundary",
  );
  const activeView = (
    await (await page.request.get(`${base}api/views`)).json()
  ).find((v) => v.name === "Browser test perspective").view;
  const rendered = await (
    await page.request.post(`${base}api/layout`, {
      data: { document: snapshot.document, view: activeView },
    })
  ).json();
  assert.deepEqual(rendered.unrouted_connections, []);
  for (const bundle of rendered.bundles)
    for (const branch of bundle.branches) {
      const n = rendered.nodes.find((n) => n.id === branch.entity);
      const port = branch.points[0];
      assert(port.x === n.position.x || port.x === n.position.x + n.width);
      assert.equal(port.y, branch.points[1].y);
    }
  const expectedPaths = rendered.bundles.flatMap((b) =>
    b.branches
      .filter((branch) => branch.points.length)
      .map((branch) =>
        branch.points.map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`).join(" "),
      ),
  );
  assert.deepEqual(
    await page
      .locator("path.route")
      .evaluateAll((paths) => paths.map((p) => p.getAttribute("d"))),
    expectedPaths,
  );
  const expectedDots = rendered.bundles.flatMap((b) =>
    b.junctions.map((p) => ({ connection: b.connections[0], x: p.x, y: p.y })),
  );
  assert.deepEqual(
    await page.locator("circle.junction").evaluateAll((dots) =>
      dots.map((dot) => ({
        connection: dot.dataset.connection,
        x: Number(dot.getAttribute("cx")),
        y: Number(dot.getAttribute("cy")),
      })),
    ),
    expectedDots,
  );
  for (const bundle of rendered.bundles)
    for (const p of bundle.junctions) {
      const arms = new Set();
      for (const branch of bundle.branches)
        for (let i = 1; i < branch.points.length; i++) {
          const a = branch.points[i - 1],
            b = branch.points[i];
          if (
            a.y === b.y &&
            Math.abs(p.y - a.y) < 1e-7 &&
            p.x >= Math.min(a.x, b.x) - 1e-7 &&
            p.x <= Math.max(a.x, b.x) + 1e-7
          ) {
            if (p.x > Math.min(a.x, b.x) + 1e-7) arms.add("left");
            if (p.x < Math.max(a.x, b.x) - 1e-7) arms.add("right");
          }
          if (
            a.x === b.x &&
            Math.abs(p.x - a.x) < 1e-7 &&
            p.y >= Math.min(a.y, b.y) - 1e-7 &&
            p.y <= Math.max(a.y, b.y) + 1e-7
          ) {
            if (p.y > Math.min(a.y, b.y) + 1e-7) arms.add("up");
            if (p.y < Math.max(a.y, b.y) - 1e-7) arms.add("down");
          }
        }
      assert(arms.size >= 3);
    }
  const segments = (b) =>
    b.branches.flatMap((branch) =>
      branch.points.slice(1).map((p, i) => [branch.points[i], p]),
    );
  for (let i = 0; i < rendered.bundles.length; i++)
    for (let j = i + 1; j < rendered.bundles.length; j++)
      for (const [a, b] of segments(rendered.bundles[i]))
        for (const [c, d] of segments(rendered.bundles[j])) {
          if (
            a.y === b.y &&
            c.y === d.y &&
            Math.max(Math.min(a.x, b.x), Math.min(c.x, d.x)) <
              Math.min(Math.max(a.x, b.x), Math.max(c.x, d.x)) - 1e-6
          )
            assert(Math.abs(a.y - c.y) >= 10 - 1e-6);
          if (
            a.x === b.x &&
            c.x === d.x &&
            Math.max(Math.min(a.y, b.y), Math.min(c.y, d.y)) <
              Math.min(Math.max(a.y, b.y), Math.max(c.y, d.y)) - 1e-6
          )
            assert(Math.abs(a.x - c.x) >= 10 - 1e-6);
        }
  for (const n of native.nodes) {
    const wasm = nested[n.id];
    assert.deepEqual(
      [n.position.x, n.position.y, n.width, n.height],
      [wasm.x, wasm.y, wasm.width, wasm.height],
    );
  }
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
  const rootHeader = await exported
    .locator('.space[data-id="house"] .space-header')
    .boundingBox();
  const oldRoot = await exported
    .locator('.space[data-id="house"]')
    .getAttribute("transform");
  await exported.mouse.move(rootHeader.x + 80, rootHeader.y + 18);
  await exported.mouse.down();
  await exported.mouse.move(rootHeader.x + 100, rootHeader.y + 28, {
    steps: 2,
  });
  await exported.mouse.up();
  await settled(exported);
  assert.notEqual(
    await exported.locator('.space[data-id="house"]').getAttribute("transform"),
    oldRoot,
  );
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
