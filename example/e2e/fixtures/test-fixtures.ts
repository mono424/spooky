import { test as base, expect, Page } from '@playwright/test';

export interface TestUser {
  username: string;
  password: string;
}

// Fixed test user — registerUser handles "already exists" by falling back to login.
// Using a stable name avoids issues with Playwright re-evaluating modules per test file.
export const testUser: TestUser = {
  username: 'e2e_testuser',
  password: 'e2e_testpass',
};

/**
 * Wait for the SolidJS app to finish initializing its DB connection.
 * The app shows "[ SYSTEM BOOT ]" / "Initializing Database..." during init,
 * then renders the Layout with a <header> element.
 */
export async function waitForAppReady(page: Page) {
  await page.waitForSelector('header', { timeout: 30_000 });
}

/**
 * Register a new user via the UI.
 * If the user already exists (signup fails), falls back to login.
 */
export async function registerUser(page: Page, user: TestUser) {
  await page.goto('/');
  await waitForAppReady(page);

  await page.getByRole('button', { name: '[ REGISTER ]' }).click();

  await page.locator('#username').fill(user.username);
  await page.locator('#password').fill(user.password);
  await page.getByRole('button', { name: '[ EXECUTE_SIGN_UP ]' }).click();

  // Wait for either success (header shows "USER: <name>") or error (CRITICAL_ERROR).
  // We check for "USER:" in the header specifically, not just the username text,
  // because the username is also visible inside the form input field.
  const result = await Promise.race([
    page.getByText('CRITICAL_ERROR').waitFor({ timeout: 15_000 }).then(() => 'error' as const),
    page.getByText(`USER: ${user.username}`, { exact: false }).waitFor({ timeout: 15_000 }).then(() => 'ok' as const),
  ]);

  if (result === 'error') {
    // User already exists — close dialog and fall back to login
    await page.locator('button[aria-label="Close"]').click();
    await loginUser(page, user);
    return;
  }
}

/**
 * Login an existing user via the UI.
 */
export async function loginUser(page: Page, user: TestUser) {
  await page.goto('/');
  await waitForAppReady(page);

  await page.getByRole('button', { name: 'Login' }).click();

  await page.locator('#username').fill(user.username);
  await page.locator('#password').fill(user.password);
  await page.getByRole('button', { name: '[ EXECUTE_LOGIN ]' }).click();

  await expect(
    page.getByText(`USER: ${user.username}`, { exact: false })
  ).toBeVisible({ timeout: 15_000 });
}

/**
 * Navigate to a thread by clicking its title in the thread list.
 */
export async function navigateToThread(page: Page, title: string) {
  await page.goto('/');
  await waitForAppReady(page);
  await page.getByText(title).first().click();
  await page.waitForURL(/\/thread\//, { timeout: 10_000 });
}

export const test = base.extend<{ testUser: TestUser }>({
  testUser: async ({}, use) => {
    await use(testUser);
  },
});

export { expect };
