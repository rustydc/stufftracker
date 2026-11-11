// Connections and ports as inspectable things with details, driven through
// the real viewer against a running server (see browser-test.mjs).
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
page.on("dialog", (d) => d.accept());
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
const settled = () =>
  page.waitForFunction(
    () => !("layoutPending" in document.querySelector("#graph").dataset),
  );
// A point on a connection's wire, in page coordinates.
const onWire = (id) =>
  page.evaluate((id) => {
    const p = [...document.querySelectorAll("path.route-hit")].find((p) =>
      p.dataset.connections.split(" ").includes(id),
    );
    const at = p.getPointAtLength(p.getTotalLength() / 2);
    const m = p.getScreenCTM();
    return {
      x: m.a * at.x + m.c * at.y + m.e,
      y: m.b * at.x + m.d * at.y + m.f,
    };
  }, id);
const editDetail = async (key, value) => {
  // The editor remembers being open; open it only if it is closed.
  if (
    (await page.locator("#inspector .details-editor").getAttribute("open")) ===
    null
  )
    await page.locator("#inspector .details-editor summary").click();
  await page.locator("#add-detail").click();
  const row = page.locator(".detail-row").last();
  await row.locator('[data-field="key"]').fill(key);
  await row.locator('[data-field="text"]').fill(value);
  await page.locator("#apply-details").click();
};
let original;
try {
  // The panel gets a breaker 1 feeding the kitchen circuit.
  const cur = await (await page.request.get(`${base}api/document`)).json();
  original = structuredClone(cur.document);
  const panel = cur.document.entities.find((e) => e.id === "panel");
  panel.faceplate = "panel";
  panel.ports = [{ id: "1", position: 1, amps: 20 }];
  cur.document.connections.find((c) => c.id === "circuit").endpoints[0].port =
    "1";
  await page.request.put(`${base}api/document`, {
    data: { revision: cur.revision, document: cur.document },
  });
  await page.goto(base + "#everything");
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );

  // Click a wire: the connection's inspector.
  await page.locator("#fit").click();
  const at = await onWire("water-feed");
  await page.mouse.click(at.x, at.y);
  await page.waitForFunction(() =>
    document
      .querySelector("#inspector .eyebrow")
      ?.textContent.includes("CONNECTION"),
  );
  assert.equal(await page.locator("#inspector h2").textContent(), "Cold water");
  assert.match(
    await page.locator("#inspector .ends").textContent(),
    /from\s*Water supply.*to\s*Kitchen sink/s,
  );
  assert.equal(
    (await page.locator("path.route.selected-wire").count()) > 0,
    true,
  );
  await editDetail("Pipe", "3/4 in PEX, blue");
  await page.locator('#connection-form [name="name"]').fill("Cold water main");
  await page.locator("#connection-form button:not([type])").click();
  // Its ends lead back to the things.
  await page.locator('#inspector [data-select-entity="tap"]').click();
  assert.equal(
    await page.locator("#inspector h2").textContent(),
    "Kitchen sink",
  );
  // ...and the thing's connection list leads to the connection.
  await page
    .locator('#inspector [data-select-connection="water-feed"]')
    .click();
  assert.equal(
    await page.locator("#inspector h2").textContent(),
    "Cold water main",
  );

  // A port, from the schedule: what it feeds, and its own details.
  await page.locator('.tree-row[data-select="panel"]').click();
  await page.locator('#inspector [data-select-port="panel:1"]').click();
  assert.match(
    await page.locator("#inspector .eyebrow").textContent(),
    /PORT · ELECTRICAL PANEL/,
  );
  assert.match(
    await page.locator("#inspector p.muted").textContent(),
    /Space 1 · 20 A/,
  );
  assert.match(
    await page.locator("#inspector .conn-list").textContent(),
    /Pendant lights, Temperature sensor/,
  );
  await editDetail("Wire", "12 AWG copper");
  // ...or by clicking its slot on the faceplate.
  await page.locator('#inspector [data-select-entity="panel"]').click();
  await page.locator("#inspector .faceplate-toggle").check();
  await settled();
  await page.locator('.tree-row[data-select="lamp"]').click();
  await page
    .locator('.node.faceplate[data-id="panel"] [data-port="1"] rect')
    .first()
    .click();
  await page.waitForFunction(() =>
    document.querySelector("#inspector .eyebrow")?.textContent.includes("PORT"),
  );
  assert.match(
    await page.locator("#inspector .details-list").textContent(),
    /Wire\s*12 AWG copper/,
  );
  assert.match(
    await page
      .locator('.node.faceplate[data-id="panel"] [data-port="1"] title')
      .textContent(),
    /Wire: 12 AWG copper/,
  );

  // Saved: the connection's and the port's details.
  await page.locator("#save-document").click();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("saved"),
  );
  const saved = (await (await page.request.get(`${base}api/document`)).json())
    .document;
  const water = saved.connections.find((c) => c.id === "water-feed");
  assert.equal(water.name, "Cold water main");
  assert.deepEqual(water.properties, [
    { key: "Pipe", type: "text", text: "3/4 in PEX, blue" },
  ]);
  assert.deepEqual(
    saved.entities.find((e) => e.id === "panel").ports[0].properties,
    [{ key: "Wire", type: "text", text: "12 AWG copper" }],
  );

  // Filters reach both: a connection's details bring its ends, a port's its thing.
  const leaves = () =>
    page.evaluate(() =>
      [...document.querySelectorAll(".node:not(.space)")]
        .map((n) => n.dataset.id)
        .sort(),
    );
  await page.locator("#filter").click();
  const cond = page.locator(".filter-row").first();
  await cond.locator('[data-field="key"]').selectOption("Pipe");
  await cond.locator('[data-field="op"]').selectOption("exists");
  await page.locator("#apply-filter").click();
  assert.deepEqual(await leaves(), ["tap", "water"]);
  await page.locator("#trace-back").click();
  await page.locator('.tree-row[data-select="panel"]').click();
  await page.locator('#inspector [data-select-port="panel:1"]').click();
  await page.locator('#inspector [data-has-key="Wire"]').click();
  assert.deepEqual(await leaves(), ["panel"]);
  await page.locator("#trace-back").click();
  assert.deepEqual(errors, []);
  console.log(
    "Links checks passed: clicking wires, connection inspector and details, ends and lists, port inspector from schedule and faceplate, port details, saving, filtering by both.",
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
