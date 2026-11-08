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
    await page.goto(url);
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
    assert.deepEqual(errors, []);
    await context.close();
  }
  console.log(
    "Mobile touch checks passed: pinch, moving focal point, remaining-finger pan, cancellation, offline viewer.",
  );
} finally {
  await browser.close();
}
