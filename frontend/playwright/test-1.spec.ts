import { test, expect } from '@playwright/test';

test('test', async ({ page }) => {
  await page.goto('http://localhost:4321/');
  await page.getByRole('textbox', { name: 'Username' }).click();
  await page.getByRole('textbox', { name: 'Username' }).fill('stel@energidata.dk');
  await page.getByRole('textbox', { name: 'Password' }).click();
  await page.getByRole('textbox', { name: 'Password' }).fill('Madball666');
  await page.getByRole('textbox', { name: 'Password' }).press('Enter');
  await page.getByRole('button', { name: 'Sign In' }).click();
  await page.getByRole('listitem').filter({ hasText: 'Partner 1' }).locator('div svg').click();
  await page.getByRole('listitem').filter({ hasText: /^Company 1_5$/ }).locator('div svg').click();
  await page.getByRole('link', { name: 'Property 1_5_42' }).click();
  await page.getByRole('button', { name: 'Stamdata' }).click();
});
