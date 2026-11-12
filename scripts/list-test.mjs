// The list (home screen): no layout until asked, a typed filter, checks that
// persist across queries, and "Diagram these" (see browser-test.mjs for the
// server).
import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_PATH
    ? { executablePath: process.env.CHROMIUM_PATH }
    : {}),
});
const base = process.env.STUFFTRACKER_URL || "http://127.0.0.1:8090/";
const errors = [];
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
page.on("pageerror", (e) => errors.push(e.message));
let original;
try {
  const cur = await (await page.request.get(`${base}api/document`)).json();
  original = structuredClone(cur.document);
  const router = cur.document.entities.find((e) => e.id === "router");
  router.properties = [
    { key: "IP address", type: "text", text: "192.168.1.1" },
    { key: "Model", type: "text", text: "UCG Max" },
  ];
  await page.request.put(`${base}api/document`, {
    data: { revision: cur.revision, document: cur.document },
  });
  const all = cur.document.entities;
  const spaces = all.filter((e) => e.kind === "space").map((e) => e.id);

  await page.goto(base);
  await page.waitForFunction(() =>
    document.querySelector("#status").textContent.includes("SQLite connected"),
  );
  // Nothing laid out or drawn: just the list.
  assert.equal(await page.locator(".node").count(), 0);
  assert.ok(await page.locator("#list").isVisible());
  assert.ok(!(await page.locator(".sidebar").isVisible()));
  const rows = () =>
    page
      .locator("#list-rows .list-row")
      .evaluateAll((r) => r.map((x) => x.dataset.row));
  const roots = all.filter((e) => !e.parent).map((e) => e.id);
  const firstLevel = all
    .filter((e) => !e.parent || roots.includes(e.parent))
    .map((e) => e.id);
  assert.deepEqual((await rows()).sort(), firstLevel.sort());
  const query = async (q) => {
    await page.locator("#list-query").fill(q);
  };
  const count = () => page.locator(".list-count").textContent();

  // kind:space matches every space; check them all.
  await query("kind:space");
  assert.match(
    await count(),
    new RegExp(`${spaces.length} of ${all.length} match`),
  );
  await page.locator('[data-list="check"]').click();
  assert.match(await count(), new RegExp(`${spaces.length} checked`));
  // A detail by key prefix; the row shows that detail.
  await query("ip:192.168");
  const matched = await page
    .locator("#list-rows .list-row:not(.context)")
    .evaluateAll((r) => r.map((x) => x.dataset.row));
  assert.deepEqual(matched, ["router"]);
  assert.match(
    await page.locator('[data-row="router"] .list-facts').textContent(),
    /IP address 192\.168\.1\.1/,
  );
  // Its spaces stay visible around it, dimmed.
  const contextRows = await page
    .locator("#list-rows .list-row.context")
    .evaluateAll((r) => r.map((x) => x.dataset.row));
  assert.ok(contextRows.length >= 1);
  await page.locator('[data-check="router"]').check();
  // Checks persist as the query changes.
  await query('"UCG max" -kind:space');
  assert.match(
    await count(),
    new RegExp(`1 of ${all.length} match · ${spaces.length + 1} checked`),
  );
  await query("is:checked");
  assert.match(await count(), new RegExp(`${spaces.length + 1} of`));
  await query("nothing-is-called-this");
  assert.match(
    await page.locator("#list-rows").textContent(),
    /Nothing matches/,
  );
  await query("");

  // Collapse and expand.
  const root = roots[0];
  await page.locator(`[data-twisty="${root}"]`).click();
  assert.ok(
    !(await rows()).some((id) => all.find((e) => e.id === id).parent === root),
  );
  await page.locator(`[data-twisty="${root}"]`).click();

  // Selecting a row inspects it.
  await query("router");
  await page.locator('[data-row="router"] .list-name').click();
  await page.waitForFunction(() =>
    document.querySelector("#inspector").textContent.includes("UCG Max"),
  );
  // A detail's key button filters the list (not the diagram).
  await page.locator('#inspector [data-has-key="IP address"]').first().click();
  assert.equal(
    await page.locator("#list-query").inputValue(),
    'has:"IP address"',
  );
  assert.equal(await page.locator(".node").count(), 0);

  // Uncheck the spaces again (a checked space brings everything inside it).
  await query("kind:space");
  await page.locator('[data-list="uncheck"]').click();
  assert.match(await count(), / · 1 checked/);
  await query('has:"IP address"');
  // Diagram the checked things only.
  await page.locator('[data-list="diagram"]').click();
  await page.waitForSelector('.node[data-id="router"]');
  const drawn = await page
    .locator(".node")
    .evaluateAll((n) => n.map((x) => x.dataset.id));
  // The checked thing, inside the spaces it sits in, and nothing else.
  assert.deepEqual(
    drawn.filter((id) => !spaces.includes(id)),
    ["router"],
  );
  assert.ok(!(await page.locator("#list").isVisible()));
  // A diagram of the checks opens without the trace bar.
  assert.ok(await page.locator("#focus-bar").isHidden());
  // Back to the list: checks and query are still there.
  await page.locator("#to-list").click();
  assert.ok(await page.locator("#list").isVisible());
  assert.match(await count(), / · 1 checked/);
  assert.equal(
    await page.locator("#list-query").inputValue(),
    'has:"IP address"',
  );
  // A trace opens as a dialog over the list: Back returns to the list with
  // checks untouched; Add puts what it shows into the checks; Keep checks
  // just those.
  await query("router");
  await page.locator('[data-row="router"] .list-name').click();
  await page.locator('#inspector [data-focus="upstream"]').click();
  await page.waitForSelector("#trace-back");
  await page.locator("#trace-back").click();
  assert.ok(await page.locator("#list").isVisible());
  assert.match(await count(), / · 1 checked/);
  await page.locator('#inspector [data-focus="both"]').click();
  await page.waitForSelector("#trace-add");
  const traced = await page
    .locator(".node")
    .evaluateAll((n) =>
      n.filter((x) => !x.classList.contains("space")).map((x) => x.dataset.id),
    );
  assert.ok(traced.length > 1, "router and its neighbours");
  await page.locator("#trace-add").click();
  assert.equal(await page.locator("#trace-add").count(), 0, "dialog closed");
  const kept = await page
    .locator(".node")
    .evaluateAll((n) =>
      n.filter((x) => !x.classList.contains("space")).map((x) => x.dataset.id),
    );
  assert.deepEqual(kept.sort(), traced.sort(), "the same things stay shown");
  assert.ok(await page.locator("#focus-bar").isHidden(), "the bar closes");
  await page.locator("#to-list").click();
  assert.match(await count(), new RegExp(` · ${traced.length} checked`));
  // From a diagram: a trace, then Keep replaces the checks with its things.
  await page.locator('[data-list="clear"]').click();
  await query("router");
  await page.locator('[data-check="router"]').check();
  await page.locator('[data-list="diagram"]').click();
  await page.locator('.node[data-id="router"]').first().click();
  await page.locator('#inspector [data-focus="both"]').click();
  await page.waitForSelector("#trace-keep");
  const up = await page
    .locator(".node")
    .evaluateAll((n) =>
      n.filter((x) => !x.classList.contains("space")).map((x) => x.dataset.id),
    );
  await page.locator("#trace-keep").click();
  await page.locator("#to-list").click();
  assert.match(await count(), new RegExp(` · ${up.length} checked`));
  await query("is:checked -kind:space");
  assert.match(await count(), new RegExp(`^${up.length} of`));
  await query("");

  // Everything is still one click away.
  await page.locator('[data-list="everything"]').click();
  await page.waitForFunction(
    (n) => document.querySelectorAll(".node").length >= n,
    all.length - spaces.length,
  );

  // Phones: the list fills the screen and a tap opens the sheet.
  const phone = await browser.newPage({
    viewport: { width: 390, height: 800 },
    isMobile: true,
    hasTouch: true,
  });
  phone.on("pageerror", (e) => errors.push(e.message));
  await phone.goto(base);
  await phone.waitForSelector("#list-rows .list-row");
  await phone.locator("#list-query").fill("router");
  await phone.locator('[data-row="router"] .list-name').tap();
  await phone.waitForSelector("#sheet:not([hidden])");
  assert.match(await phone.locator("#sheet").textContent(), /UCG Max/);
  await phone.close();
  assert.deepEqual(errors, []);
  console.log(
    "List checks passed: no layout on load, tree and folding, typed filter (kind, detail prefix, quotes, negation, is:checked), context rows, checks across queries, detail key buttons, diagram of the checked, trace Back/Add/Keep, back to the list, everything, phone sheet.",
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
