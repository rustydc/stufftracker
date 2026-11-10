import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium",
});
try {
  for (const url of [
    process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/",
    "http://stufftracker-offline.test/",
  ]) {
    const context = await browser.newContext({
      viewport: { width: 390, height: 844 },
      isMobile: true,
      hasTouch: true,
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    if (url.includes("stufftracker-offline.test"))
      await page.route("**/*", (route) =>
        route.fulfill({
          contentType: "text/html",
          body: readFileSync("work/sample-viewer.html", "utf8"),
        }),
      );
    await page.goto(url.includes("offline") ? url : url + "#everything");
    await page.waitForSelector(".node");
    const cdp = await context.newCDPSession(page);
    const frame = await page.locator("#graph").boundingBox();
    const x = frame.x + frame.width / 2,
      y = frame.y + frame.height / 2;
    const touch = async (type, points) =>
      cdp.send("Input.dispatchTouchEvent", {
        type,
        touchPoints: points.map(([id, x, y]) => ({ id, x, y })),
      });
    const camera = () =>
      page.locator("#scene").evaluate((el) => {
        const m = el.transform.baseVal.consolidate().matrix;
        return { x: m.e, y: m.f, scale: m.a };
      });
    const nodes = () =>
      page
        .locator(".node")
        .evaluateAll((ns) => ns.map((n) => n.getAttribute("transform")));
    const beforeNodes = await nodes(),
      before = await camera();
    await touch("touchStart", [
      [1, x - 30, y],
      [2, x + 30, y],
    ]);
    await touch("touchMove", [
      [1, x - 60 + 10, y + 15],
      [2, x + 60 + 10, y + 15],
    ]);
    const after = await camera();
    assert(
      Math.abs(after.scale - Math.min(2.5, before.scale * 2)) < 0.001,
      "pinch changes canvas scale",
    );
    const world = {
      x: (x - frame.x - before.x) / before.scale,
      y: (y - frame.y - before.y) / before.scale,
    };
    assert(
      Math.abs(after.x + world.x * after.scale - (x - frame.x + 10)) < 0.01,
      "zoom anchors at moving centroid",
    );
    assert(
      Math.abs(after.y + world.y * after.scale - (y - frame.y + 15)) < 0.01,
    );
    assert.deepEqual(
      await nodes(),
      beforeNodes,
      "pinch must not move entities",
    );
    await touch("touchEnd", [[1, x - 50, y + 15]]);
    await touch("touchMove", [[2, x + 80, y + 25]]);
    await page.waitForFunction(
      (previous) =>
        document.querySelector("#scene").transform.baseVal.consolidate().matrix
          .e !== previous,
      after.x,
    );
    const pan = await camera();
    assert(
      Math.abs(pan.x - after.x - 10) < 0.01,
      "remaining finger pans without a jump",
    );
    assert(Math.abs(pan.y - after.y - 10) < 0.01);
    await touch("touchCancel", []);
    assert.deepEqual(await nodes(), beforeNodes);
    // A fresh gesture after cancellation still works.
    await touch("touchStart", [
      [1, x - 20, y],
      [2, x + 20, y],
    ]);
    await touch("touchMove", [
      [1, x - 10, y],
      [2, x + 10, y],
    ]);
    assert((await camera()).scale < pan.scale);
    await touch("touchEnd", []);

    // A tap opens the thing's bottom sheet; a drag does not.
    await page.locator("#fit").click();
    const tapOn = async (id) => {
      const b = await page.locator(`.node[data-id="${id}"]`).boundingBox();
      const p = [b.x + b.width / 2, b.y + Math.min(20, b.height / 2)];
      await touch("touchStart", [[1, ...p]]);
      await touch("touchEnd", []);
    };
    const online = !url.includes("offline");
    let original;
    if (online) {
      // Give the panel ports so the sheet offers its physical layout.
      const cur = await (await page.request.get(`${url}api/document`)).json();
      original = structuredClone(cur.document);
      const panel = cur.document.entities.find((e) => e.id === "panel");
      panel.faceplate = "panel";
      panel.ports = [
        { id: "1", label: "20 A kitchen", position: 1 },
        { id: "2", label: "15 A living", position: 2 },
      ];
      await page.request.put(`${url}api/document`, {
        data: { revision: cur.revision, document: cur.document },
      });
      await page.reload();
      await page.waitForSelector(".node");
      await page.locator("#fit").click();
    }
    try {
      assert(await page.locator("#sheet").isHidden());
      await tapOn("lamp");
      await page.waitForSelector("#sheet:not([hidden])");
      assert.match(
        await page.locator("#sheet h2").textContent(),
        /Pendant lights/,
      );
      await page.locator("#sheet .sheet-close").click();
      assert(await page.locator("#sheet").isHidden());
      const lamp = await page.locator('.node[data-id="lamp"]').boundingBox();
      await touch("touchStart", [[1, lamp.x + 20, lamp.y + 20]]);
      await touch("touchMove", [[1, lamp.x + 60, lamp.y + 50]]);
      await touch("touchEnd", []);
      assert(await page.locator("#sheet").isHidden(), "a drag is not a tap");
      await tapOn("lamp");
      await page.locator('#sheet [data-focus="upstream"]').click();
      await page.waitForSelector("#focus-bar:not([hidden])");
      assert(await page.locator("#sheet").isHidden());
      const traced = await page.evaluate(() =>
        [...document.querySelectorAll(".node:not(.space)")]
          .map((n) => n.dataset.id)
          .sort(),
      );
      assert.deepEqual(traced, ["ha", "lamp", "panel", "router"]);
      await page.locator("#exit-focus").click();
      await page.locator("#fit").click();

      // Select mode: taps pick, then show only those.
      await page.locator("#select-mode").click();
      await page.waitForSelector("#pick-bar:not([hidden])");
      await tapOn("tv");
      await tapOn("desk");
      assert(await page.locator("#sheet").isHidden(), "picking opens no sheet");
      assert.match(await page.locator("#pick-bar").textContent(), /2 selected/);
      await page.locator("#focus-picked").click();
      const only = await page.evaluate(() =>
        [...document.querySelectorAll(".node:not(.space)")]
          .map((n) => n.dataset.id)
          .sort(),
      );
      assert.deepEqual(only, ["desk", "tv"]);
      assert.equal(
        await page.locator("#select-mode").getAttribute("aria-pressed"),
        "false",
      );
      await page.locator("#exit-focus").click();

      if (online) {
        await page.locator("#fit").click();
        await tapOn("panel");
        await page.locator("#sheet .faceplate-toggle").check();
        await page.waitForSelector('.node.faceplate[data-id="panel"]');
        assert.equal(
          await page.locator("#sheet .schedule tbody tr").count(),
          3,
        );
      }
    } finally {
      if (original) {
        const cur = await (await page.request.get(`${url}api/document`)).json();
        await page.request.put(`${url}api/document`, {
          data: { revision: cur.revision, document: original },
        });
      }
    }
    assert.deepEqual(errors, []);
    await context.close();
  }
  console.log(
    "Mobile touch checks passed: pinch, moving focal point, remaining-finger pan, cancellation, tap sheet, trace, select mode, faceplate toggle, offline viewer.",
  );
} finally {
  await browser.close();
}
