import { test, expect } from '@playwright/test';

// Regression: using the SAME client island twice on one page must emit exactly
// one import statement (a duplicate import is a SyntaxError) and mount BOTH
// instances — each with its OWN serialized props from SSR (data-props).
//
// Fixture: pages/Index.tsx renders <Widget label="Alpha" client:hydrate />
// followed by <Widget label="Beta" client:hydrate />. Widget is a client
// island rendering <div class="widget"><span class="widget-label">{label}</span></div>.

const BASE_URL = process.env.MARISJS_DEV_URL || 'http://127.0.0.1:3000';

test('same island used twice imports once and mounts both instances with their own props', async ({ page }) => {
  const pageErrors = [];
  page.on('pageerror', e => pageErrors.push(e.message));

  await page.goto(BASE_URL);
  await page.waitForSelector('[data-hydrate="Widget"]');

  // The page must load with NO module errors (a duplicate `import { Widget }`
  // in the page script is a SyntaxError that aborts the whole module).
  expect(pageErrors, 'page must load without script errors').toEqual([]);

  // Exactly two placeholders, one per JSX instance.
  const placeholders = page.locator('[data-hydrate="Widget"]');
  expect(await placeholders.count()).toBe(2);

  // Each instance hydrates with ITS OWN props from data-props.
  const labels = page.locator('.widget-label');
  expect(await labels.count()).toBe(2);
  await expect(labels.nth(0)).toHaveText('Alpha');
  await expect(labels.nth(1)).toHaveText('Beta');

  // The two hydrate placeholders carry distinct serialized props.
  await expect(placeholders.nth(0)).toHaveAttribute('data-props', '{"label":"Alpha"}');
  await expect(placeholders.nth(1)).toHaveAttribute('data-props', '{"label":"Beta"}');
});
