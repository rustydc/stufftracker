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
    "sensor.kitchen_thermometer_temperature": {
      state: "21.43",
      name: "Kitchen temperature",
      changed: now,
      attributes: { unit_of_measurement: "°C", device_class: "temperature" },
    },
    "device_tracker.core_switch": {
      state: "home",
      name: "Core switch",
      changed: now - 3600000,
      attributes: { source_type: "router", ip: "192.168.1.2" },
    },
    "sensor.core_switch_state": {
      state: "connected",
      name: "Core switch State",
      changed: now,
      attributes: {},
    },
    "sensor.core_switch_temperature": {
      state: "48.2",
      name: "Core switch Temperature",
      changed: now,
      attributes: { unit_of_measurement: "°C", device_class: "temperature" },
    },
    "button.core_switch_restart": {
      state: "unknown",
      name: "Restart",
      changed: now,
      attributes: { device_class: "restart" },
    },
    "switch.core_switch_port_2_poe": {
      state: "on",
      name: "Port 2 PoE",
      changed: now,
      attributes: {},
    },
    "sensor.core_switch_port_2_poe_power": {
      state: "4.21",
      name: "Port 2 PoE Power",
      changed: now,
      attributes: { unit_of_measurement: "W", device_class: "power" },
    },
    "sensor.core_switch_port_2_link_speed": {
      state: "1000",
      name: "Port 2 Link speed",
      changed: now,
      attributes: { unit_of_measurement: "Mbit/s", device_class: "data_rate" },
    },
    "sensor.core_switch_port_4_link_speed": {
      state: "0",
      name: "Port 4 Link speed",
      changed: now,
      attributes: { unit_of_measurement: "Mbit/s", device_class: "data_rate" },
    },
    "switch.core_switch_port_3_poe": {
      state: "off",
      name: "Port 3 PoE",
      changed: now,
      attributes: {},
    },
    "sensor.core_switch_port_3_poe_power": {
      state: "0",
      name: "Port 3 PoE Power",
      changed: now,
      attributes: { unit_of_measurement: "W", device_class: "power" },
    },
    "sensor.p1s_bed_temperature": {
      state: "32.0",
      name: "P1S Bed temperature",
      changed: now,
      attributes: { unit_of_measurement: "°F", device_class: "temperature" },
    },
    "light.barnap_led": {
      state: "on",
      name: "BarnAP LED",
      changed: now,
      attributes: {},
    },
    "light.barn_lights": {
      state: "off",
      name: "Barn lights",
      changed: now,
      attributes: {},
    },
    "switch.pump": {
      state: "unavailable",
      name: "Pump",
      changed: now,
      attributes: {},
    },
  },
  areas: {
    barn: {
      name: "Barn",
      entities: [
        "sensor.p1s_bed_temperature",
        "light.barnap_led",
        "light.barn_lights",
      ],
    },
    kitchen: {
      name: "Kitchen",
      entities: ["light.pendants", "sensor.kitchen_thermometer_temperature"],
    },
  },
  devices: {
    "0123456789abcdef0123456789abcdef": {
      entities: [
        { id: "device_tracker.core_switch", category: "" },
        { id: "sensor.core_switch_state", category: "diagnostic" },
        { id: "sensor.core_switch_temperature", category: "" },
        { id: "button.core_switch_restart", category: "config" },
        { id: "switch.core_switch_port_2_poe", category: "config" },
        { id: "sensor.core_switch_port_2_poe_power", category: "diagnostic" },
        { id: "switch.core_switch_port_3_poe", category: "config" },
        { id: "sensor.core_switch_port_3_poe_power", category: "diagnostic" },
      ],
      ports: {
        2: [
          "sensor.core_switch_port_2_link_speed",
          "switch.core_switch_port_2_poe",
          "sensor.core_switch_port_2_poe_power",
        ],
        4: ["sensor.core_switch_port_4_link_speed"],
        3: [
          "switch.core_switch_port_3_poe",
          "sensor.core_switch_port_3_poe_power",
        ],
      },
      disabled: [
        "sensor.core_switch_port_2_rx",
        "sensor.core_switch_port_2_tx",
      ],
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
  // An area with nothing worth saying: a printer bed, a device LED, lights off.
  set("tv", [link("home_assistant", "area", "barn")]);
  set("panel", [link("outline", "document", "doc-1")]);
  // A network switch: its HA device (ports and all) and its tracker.
  set("router", [
    link("home_assistant", "device", "0123456789abcdef0123456789abcdef"),
    link("home_assistant", "entity", "device_tracker.core_switch"),
  ]);
  const router = cur.document.entities.find((e) => e.id === "router");
  router.faceplate = "switch";
  router.ports = Array.from({ length: 16 }, (_, k) => ({
    id: String(k + 1),
    position: k + 1,
  }));
  // Port 9 is the uplink: the supply, kept in its own place.
  router.ports[8].main = true;
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
  await page.goto(base + "#everything");
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
  assert.equal(
    await page.locator('.node[data-id="tv"] .ha-pill').count(),
    0,
    "no pill without useful data",
  );
  assert.deepEqual(await pill("kitchen"), {
    text: "21.4 °C · 1 light on",
    tone: "on",
  });

  // Network gear: online (not "home") with a reading, and per-port state.
  assert.deepEqual(await pill("router"), {
    text: "online · 48.2 °C",
    tone: "value",
  });
  // Show the switch's physical layout (ports) instead of its card.
  await page.locator('.tree-row[data-select="router"]').click();
  await page.locator("#inspector .faceplate-toggle").check();
  await page.waitForSelector('.node.faceplate[data-id="router"]');
  const tag = (port) =>
    page
      .locator(`.node[data-id="router"] [data-port="${port}"] .port-tag`)
      .evaluate((g) => ({
        text: g.textContent,
        tone: [...g.classList].find((c) => c !== "port-tag"),
      }));
  // Link speed is the light's colour, so the tag only carries the draw.
  assert.deepEqual(await tag("2"), { text: "4.2 W", tone: "on" });
  const light = (port) =>
    page
      .locator(`.node[data-id="router"] [data-port="${port}"] .link-light`)
      .evaluate((g) => ({
        cls: [...g.classList].filter((c) => c !== "link-light").join(" "),
        text: g.textContent,
        bolt: !!g.querySelector(".bolt"),
      }));
  assert.deepEqual(await light("2"), {
    cls: "gbe powered",
    text: "2",
    bolt: true,
  });
  assert.deepEqual(await light("3"), {
    cls: "unknown poe",
    text: "3",
    bolt: true,
  });
  assert.deepEqual(await light("4"), { cls: "down", text: "4", bolt: false });
  assert.deepEqual(await light("5"), {
    cls: "unknown",
    text: "5",
    bolt: false,
  });
  // The uplink stays port 9 (a caret in its light), with no MAIN row.
  assert.equal(
    await page
      .locator('.node[data-id="router"] [data-port="9"] .link-light .uplink')
      .count(),
    1,
  );
  assert.doesNotMatch(
    await page.locator('.node[data-id="router"]').textContent(),
    /MAIN/,
  );
  // Sixteen ports: odd down the left, even down the right.
  const at = (port) =>
    page
      .locator(`.node[data-id="router"] [data-port="${port}"] > rect`)
      .evaluate((r) => [
        Number(r.getAttribute("x")),
        Number(r.getAttribute("y")),
      ]);
  const [x1, y1] = await at("1"),
    [x2, y2] = await at("2"),
    [x15, y15] = await at("15");
  assert.ok(x2 > x1 && y2 === y1 && x15 === x1 && y15 > y1);
  // Mirrored like a panel: the even column's light sits at its outer edge.
  const lightX = (port) =>
    page
      .locator(`.node[data-id="router"] [data-port="${port}"] .link-light rect`)
      .evaluate((r) => Number(r.getAttribute("x")));
  assert.equal((await lightX("1")) - x1, 3);
  assert.ok((await lightX("2")) - x2 > 100, "even light on the right");
  assert.deepEqual(await tag("3"), { text: "PoE off", tone: "off" });
  assert.equal(
    await page
      .locator('.node[data-id="router"] [data-port="1"] .port-tag')
      .count(),
    0,
  );
  await page.locator('.tree-row[data-select="router"]').click();
  const dev = await page.locator("#inspector .ha-device").textContent();
  assert.match(dev, /3 ports — select a port/);
  assert.match(dev, /Core switch Temperature.*48\.2 °C/s);
  assert.doesNotMatch(dev, /Port 2 PoE/, "port entities live on the port");
  await page
    .locator('.node[data-id="router"] [data-port="2"] rect')
    .first()
    .click();
  await page.waitForSelector("#inspector .port-live h3");
  const portLive = await page.locator("#inspector .port-live").textContent();
  assert.match(portLive, /Port 2 PoE Power.*4\.2 W/s);
  assert.match(portLive, /Turned off in Home Assistant: core_switch_port_2_rx/);

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
  await bare.goto(base + "#everything");
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
    "Live checks passed: drawing before status, light/door/unavailable/area pills, inspector states and attributes, online network gear, device entities, port PoE tags and inspector, Outline titles and links, unwired.",
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
