// Physical layouts (ports, faceplates, schedules) and focused views, driven
// through the real viewer against a running server (see browser-test.mjs).
import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_PATH
    ? { executablePath: process.env.CHROMIUM_PATH }
    : {}),
});
const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("dialog", (d) => d.accept("12"));
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
const settled = () =>
  page.waitForFunction(
    () => !("layoutPending" in document.querySelector("#graph").dataset),
  );
const geometry = (parent) =>
  page.evaluate(
    (parent) =>
      [...document.querySelectorAll(`.node[data-parent="${parent}"]`)].map(
        (n) => {
          const [x, y] = n
            .getAttribute("transform")
            .match(/-?[\d.]+/g)
            .map(Number);
          return {
            id: n.dataset.id,
            x,
            y,
            w: +n.dataset.width,
            h: +n.dataset.height,
          };
        },
      ),
    parent,
  );
const overlaps = (boxes) =>
  boxes.flatMap((a) =>
    boxes
      .filter(
        (b) =>
          a.id < b.id &&
          a.x < b.x + b.w &&
          b.x < a.x + a.w &&
          a.y < b.y + b.h &&
          b.y < a.y + a.h,
      )
      .map((b) => `${a.id}/${b.id}`),
  );
let original;
try {
  // A breaker schedule on the panel: a 2-pole breaker and a tandem space.
  const cur = await (await page.request.get(`${base}api/document`)).json();
  original = structuredClone(cur.document);
  const doc = cur.document;
  const panel = doc.entities.find((e) => e.id === "panel");
  panel.faceplate = "panel";
  panel.ports = [
    { id: "1", label: "20 A kitchen", position: 1 },
    { id: "2", label: "15 A living", position: 2 },
    { id: "3/5", label: "30 A dryer", position: 3, poles: 2 },
    { id: "6A", label: "15 A bath", position: 6, half: "A" },
    { id: "6B", label: "15 A hall", position: 6, half: "B" },
  ];
  doc.connections.find((c) => c.id === "circuit").endpoints[0].port = "1";
  const put = await page.request.put(`${base}api/document`, {
    data: { revision: cur.revision, document: doc },
  });
  assert.equal(put.status(), 200);
  await page.goto(base + "#everything");
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );

  // Schedule in the inspector; the faceplate replaces the card.
  await page.locator('.tree-row[data-select="panel"]').click();
  const rows = page.locator(".schedule tbody tr");
  assert.equal(await rows.count(), 6, "5 breakers + not assigned");
  assert.match(await rows.nth(2).textContent(), /3\/5.*30 A dryer.*spare/);
  await page.locator("#inspector .faceplate-toggle").check();
  await page.waitForSelector('.node.faceplate[data-id="panel"]');
  assert.equal(
    await page.locator('.node.faceplate[data-id="panel"] .slot.empty').count(),
    1,
    "space 4 is an empty slot",
  );
  assert.deepEqual(
    overlaps(await geometry("utility")),
    [],
    "neighbours pushed aside",
  );

  // Assigning a port pins the wire to that breaker's row and column edge.
  await page.locator('select[data-assign="living-power"]').selectOption("2");
  await settled();
  const pin = await page.evaluate(() => {
    const n = document.querySelector('.node.faceplate[data-id="panel"]');
    const [nx, ny] = n
      .getAttribute("transform")
      .match(/-?[\d.]+/g)
      .map(Number);
    const r = n.querySelector('[data-port="2"] rect');
    const y = ny + +r.getAttribute("y") + +r.getAttribute("height") / 2;
    const x = nx + +n.dataset.width;
    const starts = [...document.querySelectorAll(".route")].map((p) =>
      p
        .getAttribute("d")
        .match(/^M(-?[\d.]+),(-?[\d.]+)/)
        .slice(1)
        .map(Number),
    );
    return starts.some(
      ([sx, sy]) => Math.abs(sx - x) < 0.01 && Math.abs(sy - y) < 0.01,
    );
  });
  assert(pin, "living circuit leaves breaker 2 on the panel's right edge");

  // A main (supply) row and a linked breaker crossing halves, typed into
  // the editor as "main" and "B/A".
  await page.locator(".port-editor summary").click();
  for (const [id, pos, poles, half] of [
    ["MAIN", "main", "2", ""],
    ["7B/9A", "7", "2", "B/A"],
    ["7A", "7", "1", "A"],
  ]) {
    await page.locator("#add-port").click();
    const row = page.locator(".port-row[data-row]").last();
    await row.locator('[name="id"]').fill(id);
    await row.locator('[name="position"]').fill(pos);
    await row.locator('[name="poles"]').selectOption(poles);
    await row.locator('[name="half"]').fill(half);
  }
  // Ratings are port data: 20 A GFCI on 7A, typed into the editor.
  const a7 = page.locator(".port-row[data-row]").last();
  await a7.locator('[name="amps"]').fill("20");
  await a7.locator('[name="protection"]').selectOption("gfci");
  await page.locator("#ports-form button:not([type])").click();
  await page.waitForSelector(
    '.node.faceplate[data-id="panel"] [data-port="MAIN"]',
  );
  assert.match(
    await page.locator(".schedule tbody tr").first().textContent(),
    /^Main\s*MAIN/,
  );
  const linked = await page.evaluate(() => {
    const at = (id) =>
      [
        ...document.querySelectorAll(
          `.node.faceplate[data-id="panel"] [data-port="${id}"] rect`,
        ),
      ].map((r) => [+r.getAttribute("y"), +r.getAttribute("height")])[0];
    return { main: at("MAIN"), one: at("1"), b79: at("7B/9A"), a7: at("7A") };
  });
  assert(linked.main[0] < linked.one[0], "the main row sits above space 1");
  assert(linked.a7[0] + linked.a7[1] < linked.b79[0], "7B/9A starts below 7A");
  const sevenA = page.locator(
    '.node.faceplate[data-id="panel"] [data-port="7A"]',
  );
  assert.equal(
    await sevenA.locator(".amps").textContent(),
    "20",
    "rating on the handle",
  );
  assert.equal(await sevenA.locator(".badge.gfci").count(), 1);
  assert.match(
    await page
      .locator(".schedule tbody tr", { hasText: "7A" })
      .first()
      .textContent(),
    /20 A · GFCI/,
  );
  await page.locator(".port-editor summary").click();

  // Port editor: a 12-port switch, label one, assign the Ethernet uplink.
  await page.locator('.tree-row[data-select="router"]').click();
  await page.locator(".port-editor summary").click();
  await page.locator('select[name="faceplate"]').selectOption("switch");
  await page.locator("#number-ports").click();
  await page
    .locator('.port-row[data-row="2"] input[name="label"]')
    .fill("TV uplink");
  await page.locator("#ports-form button:not([type])").click();
  await page.waitForFunction(
    () => document.querySelectorAll(".schedule tbody tr").length === 13,
  );
  await page.locator('select[data-assign="ethernet"]').selectOption("3");
  // A collision is refused with a message, leaving the inventory alone (the
  // editor stays open after applying).
  await page
    .locator('.port-row[data-row="1"] input[name="position"]')
    .fill("1");
  await page.locator("#ports-form button:not([type])").click();
  await page.waitForFunction(() =>
    document
      .querySelector("#status")
      .textContent.includes("position 1 is used twice"),
  );
  await page.locator("#save-document").click();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("saved"),
  );
  const saved = (await (await page.request.get(`${base}api/document`)).json())
    .document;
  const router = saved.entities.find((e) => e.id === "router");
  assert.equal(router.faceplate, "switch");
  assert.equal(router.ports.length, 12);
  assert.equal(router.ports[2].label, "TV uplink");
  assert.equal(
    saved.connections
      .find((c) => c.id === "ethernet")
      .endpoints.find((e) => e.entity === "router").port,
    "3",
  );

  // Focus: trace what feeds the lamp, then widen and leave it.
  const before = await geometry("kitchen");
  const utilityBefore = await geometry("utility");
  await page.locator('.tree-row[data-select="lamp"]').click();
  await page.locator('[data-focus="upstream"]').click();
  await page.waitForSelector("#focus-bar:not([hidden])");
  const shown = await page.evaluate(() =>
    [...document.querySelectorAll(".node:not(.space)")]
      .map((n) => n.dataset.id)
      .sort(),
  );
  assert.deepEqual(shown, ["ha", "lamp", "panel", "router"]);
  await page.locator('#focus-bar [data-dir="both"]').click();
  await page.locator('#focus-bar [data-hops="1"]').click();
  assert.equal(
    await page.locator('.node[data-id="sensor"]').count(),
    1,
    "one hop either way reaches the sensor",
  );
  // Things can be arranged within the focus; the arrangement is the focus's.
  const sensorAt = async () =>
    (await geometry("kitchen")).find((n) => n.id === "sensor");
  const s0 = await sensorAt();
  const box = await page.locator('.node[data-id="sensor"]').boundingBox();
  await page.mouse.move(box.x + 30, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 30, box.y + 120, { steps: 5 });
  await page.mouse.up();
  await settled();
  const s1 = await sensorAt();
  assert(s1.y > s0.y + 50, "the sensor moved within the focus");
  // Widening the focus keeps that arrangement.
  await page.locator('#focus-bar [data-hops="2"]').click();
  assert.equal((await sensorAt()).y, s1.y);
  await page.locator("#exit-focus").click();
  assert(await page.locator("#focus-bar").isHidden());
  assert.deepEqual(
    await geometry("kitchen"),
    before,
    "focus left the layout alone",
  );
  assert.deepEqual(await geometry("utility"), utilityBefore);

  // Shift-click picks, then show only those.
  await page.keyboard.down("Shift");
  await page.locator('.node[data-id="tv"]').click();
  await page.locator('.node[data-id="desk"]').click();
  await page.keyboard.up("Shift");
  assert.match(await page.locator("#pick-bar").textContent(), /2 selected/);
  await page.locator("#focus-picked").click();
  const only = await page.evaluate(() =>
    [...document.querySelectorAll(".node:not(.space)")]
      .map((n) => n.dataset.id)
      .sort(),
  );
  assert.deepEqual(only, ["desk", "tv"]);
  assert(await page.locator("#pick-bar").isHidden());
  await page.locator("#exit-focus").click();
  assert.deepEqual(errors, []);
  console.log(
    "Physical checks passed: breaker schedule, faceplates, port pins, mains and linked half-space breakers, port editor, focus and trace, arranging within a focus, picking.",
  );
} finally {
  if (original) {
    const cur = await (await page.request.get(`${base}api/document`)).json();
    await page.request.put(`${base}api/document`, {
      data: { revision: cur.revision, document: original },
    });
  }
  await browser.close();
}
