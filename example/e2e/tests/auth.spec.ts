import {
  test,
  expect,
  registerUser,
  loginUser,
  testUser,
  waitForAppReady,
} from '../fixtures/test-fixtures';

test.describe.serial('Authentication flows', () => {
  test('should show landing page when unauthenticated', async ({ page }) => {
    await page.goto('/');
    await waitForAppReady(page);

    await expect(page.getByText('Welcome to Thread App')).toBeVisible();
    await expect(
      page.getByRole('button', { name: '[ INITIALIZE_SESSION ]' })
    ).toBeVisible();
    await expect(
      page.getByRole('button', { name: '[ REGISTER ]' })
    ).toBeVisible();
  });

  test('should register a new user', async ({ page }) => {
    await registerUser(page, testUser);

    // Authenticated view shows the thread list
    await expect(
      page.getByRole('button', { name: '[ + WRITE_NEW ]' })
    ).toBeVisible();

    // Landing page is gone
    await expect(page.getByText('Welcome to Thread App')).not.toBeVisible();
  });

  test('should logout and login again', async ({ page }) => {
    // Login (user already registered in previous test, fresh browser context)
    await loginUser(page, testUser);

    // Logout
    await page.getByRole('button', { name: '<< LOGOUT' }).click();
    await expect(page.getByText('Welcome to Thread App')).toBeVisible({
      timeout: 10_000,
    });

    // Login with same credentials
    await loginUser(page, testUser);

    await expect(
      page.getByRole('button', { name: '[ + WRITE_NEW ]' })
    ).toBeVisible();
  });

  test('should toggle between signup and login modes', async ({ page }) => {
    await page.goto('/');
    await waitForAppReady(page);

    // Open dialog in signup mode
    await page.getByRole('button', { name: '[ REGISTER ]' }).click();
    await expect(
      page.getByRole('button', { name: '[ EXECUTE_SIGN_UP ]' })
    ).toBeVisible();
    await expect(page.getByText('Access_Existing_Account')).toBeVisible();

    // Toggle to login mode
    await page.getByText('Access_Existing_Account').click();
    await expect(
      page.getByRole('button', { name: '[ EXECUTE_LOGIN ]' })
    ).toBeVisible();
    await expect(page.getByText('Create_New_Identifier')).toBeVisible();

    // Toggle back to signup mode
    await page.getByText('Create_New_Identifier').click();
    await expect(
      page.getByRole('button', { name: '[ EXECUTE_SIGN_UP ]' })
    ).toBeVisible();
  });
});
