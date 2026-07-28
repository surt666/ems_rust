import { test, expect } from "@playwright/test";

// End-to-end check of the two things a build cannot prove: that the de-Reacted
// dashboard actually draws its charts, and that the relocated formula editor
// opens and loads.
//
// Runs against the deployed site, not the local preview: the hierarchy API is
// reached through CloudFront's /hierarchy/* proxy, which localhost:4321 does not
// provide, and every API now needs a Cognito token.
const SITE = process.env.EMS_SITE ?? "https://d24beiqs2cj89y.cloudfront.net";
const USER = process.env.EMS_USER!;
const PASS = process.env.EMS_PASS!;
const NODE_ID = process.env.EMS_NODE ?? "HN2#10003";
const NODE_PATH = process.env.EMS_NODE_PATH ?? "HN0#root|HN1#10001";

test.use({ baseURL: SITE });

async function login(page) {
  await page.goto("/");
  await page.fill("#username", USER);
  await page.fill("#password", PASS);
  await page.click("#loginButton");
  // Amplify redirects to the shell once the session is stored.
  await page.waitForURL((u) => !u.pathname.match(/^\/$|^\/index/), { timeout: 30_000 });
}

/** The dashboard resolves its node from sessionStorage, as every widget does. */
async function selectNode(page) {
  await page.evaluate(
    ([id, path]) => {
      sessionStorage.setItem("selectedNodeId", id);
      sessionStorage.setItem("selectedNodePath", path);
      sessionStorage.setItem("selectedCompanyId", id);
    },
    [NODE_ID, NODE_PATH],
  );
}

test("node dashboard draws its charts with no React", async ({ page }) => {
  const failures: string[] = [];
  page.on("pageerror", (e) => failures.push(`pageerror: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && failures.push(`console: ${m.text()}`));

  await login(page);
  await selectNode(page);
  await page.goto(`/node/?id=${encodeURIComponent(NODE_ID)}`);

  // Charts are canvas; ECharts renders into one per container.
  await expect(page.locator("#resource-cards canvas").first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("#node-chart canvas")).toBeVisible({ timeout: 30_000 });
  await expect(page.locator("#cost-chart canvas")).toBeVisible({ timeout: 30_000 });

  // The server-rendered fragments (no canvas — they are markup).
  await expect(page.locator(".bm-bar__track").first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(".al-counts")).toBeVisible({ timeout: 30_000 });

  // Nothing React-shaped survived.
  expect(await page.locator("astro-island").count()).toBe(0);
  expect(failures, `page errors:\n${failures.join("\n")}`).toEqual([]);
});

test("formula editor opens from the node page into the schema designer", async ({ page }) => {
  await login(page);
  await selectNode(page);
  await page.goto(`/node/?id=${encodeURIComponent(NODE_ID)}`);

  // The Formler section is admin-gated; skip cleanly rather than fail if the
  // probe user is not an admin, so the result is never falsely green.
  const button = page.locator("#open-formulas");
  if (!(await button.count())) test.skip(true, "probe user is not an admin — section not rendered");

  await expect(page.locator("#formula-list")).toBeHidden();
  await button.click();

  await expect(page.locator("#sd-formulas")).toBeVisible();
  await expect(page.locator("#sd-formula-node")).toHaveText(NODE_ID);
  // The fragment is server-rendered; either it lists cards or offers a new one.
  await expect(page.locator("#formula-list .formula-card").first()).toBeVisible({ timeout: 30_000 });
});
