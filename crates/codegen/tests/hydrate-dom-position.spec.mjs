import { test, expect } from '@playwright/test';

// This test verifies that client-hydrated islands render INSIDE their correct
// <div data-hydrate="X"> placeholders, not appended at the end of the root
// container — the bug shipped in 0.1.0 and 0.1.1.
//
// The server is expected to be running at MARISJS_DEV_URL (e.g.
// http://127.0.0.1:<port>) with a fixture containing two islands
// (WidgetA, WidgetB) separated by static content (<h1>Before, <hr/>, <h2>After).

const BASE_URL = process.env.MARISJS_DEV_URL || 'http://127.0.0.1:3000';

test('hydrated islands occupy their correct DOM position between sibling static content', async ({ page }) => {
  await page.goto(BASE_URL);
  await page.waitForSelector('[data-hydrate]');

  // Get the root container's direct children (the prerendered wrapper div inside #root)
  const children = page.locator('#root > div > *');

  const count = await children.count();
  expect(count, 'page should have at least 5 top-level children (h1, WidgetA, hr, WidgetB, h2)').toBeGreaterThanOrEqual(5);

  // Child 0: the static <h1>Before</h1>
  await expect(children.nth(0)).toHaveText('Before');
  expect(await children.nth(0).evaluate(el => el.tagName)).toBe('H1');

  // Child 1: <div data-hydrate="WidgetA"> — must contain WidgetA's rendered <span class="first">First</span>
  const widgetA = children.nth(1);
  await expect(widgetA).toHaveAttribute('data-hydrate', 'WidgetA');
  const widgetAFirst = widgetA.locator('.first');
  await expect(widgetAFirst).toHaveText('First');

  // Child 2: the static <hr/>
  expect(await children.nth(2).evaluate(el => el.tagName)).toBe('HR');

  // Child 3: <div data-hydrate="WidgetB"> — must contain WidgetB's rendered <span class="second">Second</span>
  const widgetB = children.nth(3);
  await expect(widgetB).toHaveAttribute('data-hydrate', 'WidgetB');
  const widgetBSecond = widgetB.locator('.second');
  await expect(widgetBSecond).toHaveText('Second');

  // Child 4: the static <h2>After</h2>
  await expect(children.nth(4)).toHaveText('After');
  expect(await children.nth(4).evaluate(el => el.tagName)).toBe('H2');
});
