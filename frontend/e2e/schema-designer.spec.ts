import { test, expect } from "@playwright/test";

test("designer loads preset, blocks cycles, captures serialized schema", async ({ page }) => {
  await page.goto("/dev/schema-designer");
  await page.click("#open");

  // Preset types present
  await expect(page.locator(".sd-typeitem", { hasText: "building" })).toBeVisible();

  // Select "group"; building is an allowed child of group in the preset → checked
  await page.locator(".sd-typeitem", { hasText: "group" }).click();
  const buildingChk = page.locator(".sd-chk", { hasText: "building" }).locator("input[type=checkbox]");
  await expect(buildingChk).toBeChecked();

  // Select "building": "group" appears disabled (cycle: group→building exists)
  await page.locator(".sd-typeitem", { hasText: "building" }).first().click();
  const groupChk = page.locator(".sd-chk", { hasText: "group" }).locator("input[type=checkbox]");
  await expect(groupChk).toBeDisabled();

  // Done emits the schema
  await page.click("button:has-text('Done')");
  const schema = await page.evaluate(() => (window as any).__lastSchema);
  expect(schema.version).toBe(2);
  expect(schema.edges.company).toHaveProperty("group");
  expect(schema.metadata.building.lat).toMatchObject({ type: "number", required: true, min: -90, max: 90 });
  expect(schema.sensors).toContain("building");
});
