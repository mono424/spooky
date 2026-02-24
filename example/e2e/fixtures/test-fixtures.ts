import { test as base, expect, Page } from '@playwright/test';

const TEST_RUN_ID = Date.now().toString(36);

export interface TestUser {
  username: string;
  password: string;
}

export const testUser: TestUser = {
  username: `e2e_${TEST_RUN_ID}`,
  password: `pass_${TEST_RUN_ID}`,
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
 */
export async function registerUser(page: Page, user: TestUser) {
  await page.goto('/');
  await waitForAppReady(page);

  await page.getByRole('button', { name: '[ REGISTER ]' }).click();

  await page.locator('#username').fill(user.username);
  await page.locator('#password').fill(user.password);
  await page.getByRole('button', { name: '[ EXECUTE_SIGN_UP ]' }).click();

  await expect(
    page.getByText(user.username, { exact: false })
  ).toBeVisible({ timeout: 15_000 });
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
    page.getByText(user.username, { exact: false })
  ).toBeVisible({ timeout: 15_000 });
}

export const test = base.extend<{ testUser: TestUser }>({
  testUser: async ({}, use) => {
    await use(testUser);
  },
});

export { expect };
