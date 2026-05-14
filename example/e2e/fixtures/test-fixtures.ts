import type { Page } from '@playwright/test';
import { test as base, expect } from '@playwright/test';

export interface TestUser {
  username: string;
  password: string;
}

// Fixed test user — registerUser handles "already exists" by falling back
// to login. Stable name avoids issues with Playwright re-evaluating
// modules per test file.
export const testUser: TestUser = {
  username: 'e2e_testuser',
  password: 'e2e_testpass',
};

/**
 * Wait for the SolidJS app to finish initializing its DB connection.
 * Boot screen shows a "Connecting..." spinner; once the layout mounts the
 * <header> appears. This is the universal "app is interactive" gate.
 */
export async function waitForAppReady(page: Page) {
  await page.waitForSelector('header', { timeout: 30_000 });
}

/**
 * Wait for the authenticated layout — the "New thread" button is the
 * cheapest unambiguous signal that auth + thread list rendered.
 */
async function waitForAuthenticated(page: Page) {
  await expect(page.getByRole('button', { name: 'New thread' }))
    .toBeVisible({ timeout: 15_000 });
}

/**
 * Register a new user via the UI. If the username already exists (signup
 * fails), close the dialog and fall back to login with the same creds.
 */
export async function registerUser(page: Page, user: TestUser) {
  await page.goto('/');
  await waitForAppReady(page);

  // Two "Sign up" entry points exist (header + landing "Create account");
  // grab the header button by its exact label.
  await page.getByRole('button', { name: 'Sign up', exact: true }).click();

  const form = page.locator('form').filter({ has: page.locator('#username') });
  await form.locator('#username').fill(user.username);
  await form.locator('#password').fill(user.password);
  await form.getByRole('button', { name: 'Create account', exact: true }).click();

  // Race "auth succeeded" vs "username taken" (or any other error
  // surfaced inside the dialog). The success signal is the dialog
  // closing AND the thread list mounting.
  const result = await Promise.race([
    waitForAuthenticated(page).then(() => 'ok' as const),
    page
      .locator('div.bg-red-500\\/10') // error banner inside the dialog
      .waitFor({ timeout: 15_000 })
      .then(() => 'error' as const),
  ]);

  if (result === 'error') {
    await page.locator('button[aria-label="Close"]').click();
    await loginUser(page, user);
  }
}

/**
 * Login an existing user via the UI. Multiple "Sign in" buttons can be
 * visible when logged out (header + landing + dialog submit), so we
 * scope dialog interactions to the form once it's mounted.
 */
export async function loginUser(page: Page, user: TestUser) {
  await page.goto('/');
  await waitForAppReady(page);

  // Header "Sign in" — first() pins the header button (mounts before
  // landing copy renders). Landing has its own "Sign in" but tapping
  // either opens the same dialog.
  await page.getByRole('button', { name: 'Sign in', exact: true }).first().click();

  // Scope all dialog work to the form so no "Sign in" elsewhere on the
  // page can accidentally match.
  const form = page.locator('form').filter({ has: page.locator('#username') });
  await form.locator('#username').fill(user.username);
  await form.locator('#password').fill(user.password);
  await form.getByRole('button', { name: 'Sign in', exact: true }).click();

  await waitForAuthenticated(page);
}

/**
 * Sign out via the header button. After click the landing screen
 * ("Welcome to Threads") should be visible again.
 */
export async function signOutUser(page: Page) {
  await page.getByRole('button', { name: 'Sign out' }).click();
  await expect(page.getByText('Welcome to Threads'))
    .toBeVisible({ timeout: 10_000 });
}

/**
 * Navigate to a thread by clicking its card in the list. Finds the card
 * via `[data-thread-index]` filtered by the title text so we hit the
 * element that owns the `onMouseDown` handler (clicking the heading
 * directly sometimes doesn't reach the parent's mousedown via
 * Playwright). Title-based filtering also keeps the helper correct when
 * the DB has multiple threads — the alphabetical sort can put any of
 * them at index 0.
 */
export async function navigateToThread(page: Page, title: string) {
  await page.goto('/');
  await waitForAppReady(page);
  await expect(page.getByText(title, { exact: true })).toBeVisible({
    timeout: 15_000,
  });
  const card = page
    .locator('[data-thread-index]')
    .filter({ has: page.getByText(title, { exact: true }) });
  await card.first().click();
  await page.waitForURL(/\/thread\//, { timeout: 10_000 });
}

/**
 * Poll the upstream SurrealDB SQL endpoint until a thread with `title`
 * appears, or throw after `attempts * 500ms`. Used as a deterministic
 * gate after `Publish` so subsequent UI assertions (home list, sidebar,
 * second-client view) don't race the create-flow's debounced sync-up.
 */
export async function waitForThreadInUpstream(
  page: Page,
  title: string,
  attempts = 30
) {
  for (let i = 0; i < attempts; i++) {
    const resp = await fetch('http://localhost:8666/sql', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'surreal-ns': 'main',
        'surreal-db': 'example',
        Authorization:
          'Basic ' + Buffer.from('root:root').toString('base64'),
      },
      body: 'SELECT id, title FROM thread',
    });
    const body = (await resp.json()) as Array<{
      result?: Array<{ title?: string }>;
    }>;
    const rows = body[0]?.result ?? [];
    if (rows.some((r) => r.title === title)) return;
    await page.waitForTimeout(500);
  }
  throw new Error(
    `Thread "${title}" never appeared in upstream after ${attempts * 500}ms.`
  );
}

/**
 * Type into a Tiptap/ProseMirror editor identified by `selector`. The
 * editor is `[contenteditable]`, so `.fill()` doesn't work — we click to
 * focus then `.type()` keystrokes so LoroSyncPlugin captures each one.
 */
export async function typeIntoEditor(page: Page, selector: string, text: string) {
  const editor = page.locator(selector);
  await editor.click();
  await editor.press('ControlOrMeta+a');
  await editor.press('Delete');
  await page.keyboard.type(text);
}

export const test = base.extend<{ testUser: TestUser }>({
  // eslint-disable-next-line no-empty-pattern
  testUser: async ({}, use) => {
    await use(testUser);
  },
});

export { expect };
