// Details (key/value properties with uploaded attachments) driven through the
// real viewer against a running server (see browser-test.mjs).
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
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
// A 2x2 PNG and a tiny PDF.
const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==",
  "base64",
);
const pdf = Buffer.from("%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n");
let original;
try {
  original = (await (await page.request.get(`${base}api/document`)).json())
    .document;
  await page.goto(base + "#everything");
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  await page.locator('.tree-row[data-select="router"]').click();
  await page.locator(".details-editor summary").click();
  const row = (i) => page.locator(".detail-row").nth(i);
  // Text, link, image and PDF details.
  await page.locator("#add-detail").click();
  await row(0).locator('[data-field="key"]').fill("IP address");
  await row(0).locator('[data-field="text"]').fill("192.168.1.1");
  await page.locator("#add-detail").click();
  await row(1).locator('[data-field="key"]').fill("Product page");
  await row(1).locator('[data-field="type"]').selectOption("link");
  await row(1).locator('[data-field="url"]').fill("https://example.com/us8");
  await row(1).locator('[data-field="title"]').fill("Ubiquiti US-8");
  for (const [i, key, name, mimeType, buffer] of [
    [2, "Photo", "rack.png", "image/png", png],
    [3, "Manual", "US8 manual.pdf", "application/pdf", pdf],
  ]) {
    await page.locator("#add-detail").click();
    await row(i).locator('[data-field="key"]').fill(key);
    await row(i).locator('[data-field="type"]').selectOption("file");
    await row(i)
      .locator('[data-field="file"]')
      .setInputFiles({ name, mimeType, buffer });
    await page.waitForFunction(
      ([i, name]) =>
        document.querySelectorAll(".detail-row")[i]?.textContent.includes(name),
      [i, name],
    );
  }
  await page.locator("#apply-details").click();
  await page.locator("#save-document").click();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("saved"),
  );
  const saved = (await (await page.request.get(`${base}api/document`)).json())
    .document;
  const router = saved.entities.find((e) => e.id === "router");
  assert.deepEqual(
    router.properties.map((p) => [p.key, p.type]),
    [
      ["IP address", "text"],
      ["Product page", "link"],
      ["Photo", "file"],
      ["Manual", "file"],
    ],
  );
  assert.equal(router.properties[0].text, "192.168.1.1");
  const [photo, manual] = router.properties.slice(2);
  assert.equal(photo.size, png.length);

  // Shown in the details: the text, the link, the picture, the manual.
  const list = page.locator("#inspector .details-list");
  assert.match(await list.textContent(), /IP address\s*192\.168\.1\.1/);
  assert.equal(
    await list.locator('a[href="https://example.com/us8"]').textContent(),
    "Ubiquiti US-8",
  );
  await page.waitForFunction(
    () =>
      document.querySelector("#inspector .detail-image img")?.naturalWidth ===
      2,
  );
  assert.match(
    await list.locator(".detail-file").textContent(),
    /US8 manual\.pdf/,
  );
  const served = await page.request.get(
    `${base}api/attachments/${manual.attachment}`,
  );
  assert.equal(served.headers()["content-type"], "application/pdf");
  assert.deepEqual(Buffer.from(await served.body()), pdf);
  const img = await page.request.get(
    `${base}api/attachments/${photo.attachment}`,
  );
  assert.equal(img.headers()["x-content-type-options"], "nosniff");
  assert.match(img.headers()["content-security-policy"], /sandbox/);

  // Search matches details, not just names.
  await page.locator("#search").fill("192.168.1.1");
  assert.deepEqual(
    await page
      .locator(".tree-row")
      .evaluateAll((rs) => rs.map((r) => r.dataset.select)),
    ["router"],
  );
  await page.locator("#search").fill("");

  // Filters: show only the things whose details match.
  const leaves = () =>
    page.evaluate(() =>
      [...document.querySelectorAll(".node:not(.space)")]
        .map((n) => n.dataset.id)
        .sort(),
    );
  await page.locator("#filter").click();
  const cond = page.locator(".filter-row").first();
  await cond.locator('[data-field="key"]').selectOption("IP address");
  await cond.locator('[data-field="op"]').selectOption("exists");
  assert.equal(await page.locator(".filter-count").textContent(), "1 match");
  await page.locator("#apply-filter").click();
  await page.waitForSelector("#focus-bar:not([hidden])");
  assert.match(
    await page.locator("#focus-bar .focus-label").textContent(),
    /Matching has IP address/,
  );
  assert.deepEqual(await leaves(), ["router"]);
  assert.equal(
    await page.locator('#focus-bar [data-hops="0"]').getAttribute("class"),
    "on",
  );
  await page.locator('#focus-bar [data-hops="1"]').click();
  assert((await leaves()).length > 3, "one hop brings the router's neighbours");
  await page.locator("#exit-focus").click();
  // Typing a value counts matches live; "contains" on any field.
  await page.locator("#filter").click();
  await page.locator('.filter-row [data-field="value"]').fill("ubiquiti");
  assert.equal(await page.locator(".filter-count").textContent(), "1 match");
  await page.locator('.filter-row [data-field="value"]').fill("zzz");
  assert.equal(await page.locator(".filter-count").textContent(), "0 matches");
  assert(await page.locator("#apply-filter").isDisabled());
  await page.locator(".filter-close").click();
  // From the details: a key shows everything with it, ⧉ everything with
  // the same value.
  await page.locator('.tree-row[data-select="router"]').click();
  await page.locator('#inspector [data-has-key="IP address"]').click();
  assert.deepEqual(await leaves(), ["router"]);
  await page.locator("#exit-focus").click();
  await page.locator('.tree-row[data-select="router"]').click();
  await page.locator('#inspector [data-same-key="IP address"]').click();
  assert.match(
    await page.locator("#focus-bar .focus-label").textContent(),
    /IP address is "192.168.1.1"/,
  );
  // A filtered view saves and comes back.
  await page.locator("#save-view").click();
  await page.locator("#view-name").fill("Has an IP");
  await page.locator("#view-form button.primary").click();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("View saved"),
  );
  await page.locator("#exit-focus").click();
  await page.reload();
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  await page.locator("#views").selectOption({ label: "Has an IP" });
  await page.waitForSelector("#focus-bar:not([hidden])");
  assert.deepEqual(await leaves(), ["router"]);
  await page.locator("#exit-focus").click();
  // The inventory search can put its matches on the canvas.
  await page.locator("#search").fill("192.168");
  await page.locator("#search-show").click();
  assert.deepEqual(await leaves(), ["router"]);
  await page.locator("#exit-focus").click();
  await page.locator("#search").fill("");
  for (const v of await (await page.request.get(`${base}api/views`)).json())
    if (v.name === "Has an IP")
      await page.request.delete(`${base}api/views/${v.id}`);

  // A read-only share sees the details but no editor.
  const ro = await browser.newPage();
  await ro.route("**/api/session", (r) =>
    r.fulfill({ contentType: "application/json", body: '{"writable":false}' }),
  );
  await ro.goto(base + "#everything");
  await ro.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("Read-only share"),
  );
  await ro.locator('.tree-row[data-select="router"]').click();
  assert.equal(await ro.locator("#inspector .details-list dt").count(), 4);
  assert.equal(await ro.locator(".details-editor").count(), 0);
  await ro.close();
  assert.deepEqual(errors, []);
  console.log(
    "Details checks passed: text, link, image and PDF details, upload and save, serving headers, filters (panel, keys, values, hops, saved view), search, read-only view.",
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
