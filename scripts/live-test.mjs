// Live status (Home Assistant states, Outline titles) in the viewer, with the
// grain's status endpoints stubbed (see browser-test.mjs for the server).
import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_PATH
    ? { executablePath: process.env.CHROMIUM_PATH }
    : {}),
});
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
const now = Date.now();
const status = {
  connected: true,
  updated: now,
  entities: {
    "light.pendants": {
      state: "on",
      name: "Pendants",
      changed: now - 120000,
      attributes: { brightness: 200 },
    },
    "binary_sensor.back_door": {
      state: "on",
      name: "Back door",
      changed: now - 30000,
      attributes: { device_class: "door" },
    },
    "sensor.kitchen_temp": {
      state: "21.43",
      name: "Kitchen temperature",
      changed: now,
      attributes: { unit_of_measurement: "°C", device_class: "temperature" },
    },
    "switch.pump": {
      state: "unavailable",
      name: "Pump",
      changed: now,
      attributes: {},
    },
  },
  areas: {
    kitchen: {
      name: "Kitchen",
      entities: ["light.pendants", "sensor.kitchen_temp"],
    },
  },
};
const documents = {
  connected: true,
  documents: {
    "doc-1": {
      title: "Electrical panel schedule",
      url: "https://outline.example/doc/panel-abc",
    },
  },
};
const link = (provider, kind, id) => ({ provider, kind, external_id: id });
const errors = [];
const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
page.on("pageerror", (e) => errors.push(e.message));
let original;
try {
  const cur = await (await page.request.get(`${base}api/document`)).json();
  original = structuredClone(cur.document);
  const set = (id, links) =>
    (cur.document.entities.find((e) => e.id === id).links = links);
  set("kitchen", [link("home_assistant", "area", "kitchen")]);
  set("lamp", [link("home_assistant", "entity", "light.pendants")]);
  set("sensor", [link("home_assistant", "entity", "binary_sensor.back_door")]);
  set("water", [link("home_assistant", "entity", "switch.pump")]);
  set("panel", [link("outline", "document", "doc-1")]);
  await page.request.put(`${base}api/document`, {
    data: { revision: cur.revision, document: cur.document },
  });
  // Status answers late: the page must draw without waiting for it.
  let release;
  const held = new Promise((r) => (release = r));
  await page.route("**/api/live/status", async (r) => {
    await held;
    await r.fulfill({
      contentType: "application/json",
      body: JSON.stringify(status),
    });
  });
  await page.route("**/api/live/documents", (r) =>
    r.fulfill({
      contentType: "application/json",
      body: JSON.stringify(documents),
    }),
  );
  await page.goto(base);
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  assert.equal(await page.locator(".node").count(), 17, "drawn before status");
  assert.equal(await page.locator(".ha-pill").count(), 0);
  release();
  await page.waitForSelector(".ha-pill");
  const pill = (id) =>
    page.locator(`.node[data-id="${id}"] .ha-pill`).evaluate((g) => ({
      text: g.textContent,
      tone: [...g.classList].find((c) => c !== "ha-pill"),
    }));
  assert.deepEqual(await pill("lamp"), { text: "on 78%", tone: "on" });
  assert.deepEqual(await pill("sensor"), { text: "open", tone: "alert" });
  assert.deepEqual(await pill("water"), { text: "unavailable", tone: "muted" });
  assert.deepEqual(await pill("kitchen"), {
    text: "21.4 °C · 1 light on",
    tone: "on",
  });

  // The inspector: Home Assistant rows and Outline documents.
  await page.locator('.tree-row[data-select="lamp"]').click();
  const ha = page.locator("#inspector .ha-links");
  assert.match(
    await ha.textContent(),
    /Pendants.*light\.pendants.*changed 2 min ago.*brightness: 200.*on 78%/s,
  );
  await page.locator('.tree-row[data-select="panel"]').click();
  const doc = page.locator("#inspector .doc-links a");
  assert.equal(
    await doc.getAttribute("href"),
    "https://outline.example/doc/panel-abc",
  );
  assert.match(await doc.textContent(), /Electrical panel schedule/);
  assert.equal(
    await page.locator("#inspector .external-link").count(),
    0,
    "no raw IDs",
  );

  // Not wired: a plain note, no pills, nothing breaks.
  const bare = await browser.newPage({
    viewport: { width: 1500, height: 950 },
  });
  bare.on("pageerror", (e) => errors.push(e.message));
  await bare.route("**/api/live/*", (r) =>
    r.fulfill({ contentType: "application/json", body: '{"connected":false}' }),
  );
  await bare.goto(base);
  await bare.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  await bare.locator('.tree-row[data-select="lamp"]').click();
  await bare.waitForFunction(() =>
    document
      .querySelector("#inspector .ha-links")
      ?.textContent.includes("isn't connected"),
  );
  assert.equal(await bare.locator(".ha-pill").count(), 0);
  await bare.close();
  assert.deepEqual(errors, []);
  console.log(
    "Live checks passed: drawing before status, light/door/unavailable/area pills, inspector states and attributes, Outline titles and links, unwired.",
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
