import {
  test,
  expect,
  registerUser,
  loginUser,
  testUser,
  waitForAppReady,
  navigateToThread,
} from '../fixtures/test-fixtures';

test.describe.serial('Thread CRUD operations', () => {
  const threadTitle = 'E2E Test Thread';
  const threadContent = 'Automated test content for e2e testing';
  const updatedTitle = 'Updated E2E Test Thread';
  const commentContent = 'E2E automated test comment';

  test('should create a new thread', async ({ page }) => {
    await registerUser(page, testUser);

    // Click "[ + WRITE_NEW ]" to navigate to /create-thread
    await page.getByRole('button', { name: '[ + WRITE_NEW ]' }).click();

    // Wait for the create thread dialog
    await expect(page.locator('#title')).toBeVisible({ timeout: 10_000 });

    // Fill in the form
    await page.locator('#title').fill(threadTitle);
    await page.locator('#content').fill(threadContent);

    // Submit
    await page.getByRole('button', { name: '[ PUBLISH_THREAD ]' }).click();

    // Wait for navigation to /thread/:id
    await page.waitForURL(/\/thread\//, { timeout: 15_000 });

    // Verify thread detail page (author sees editable inputs)
    await expect(
      page.locator('input[placeholder="UNTITLED_THREAD"]')
    ).toHaveValue(threadTitle, { timeout: 15_000 });

    await expect(page.getByText('MODE: READ_WRITE')).toBeVisible();

    await expect(
      page.getByText(`AUTHOR: ${testUser.username}`, { exact: false })
    ).toBeVisible();
  });

  test('should update thread title', async ({ page }) => {
    await loginUser(page, testUser);

    // Navigate to the thread via the thread list
    await navigateToThread(page, threadTitle);

    const titleInput = page.locator('input[placeholder="UNTITLED_THREAD"]');
    await expect(titleInput).toBeVisible({ timeout: 15_000 });

    // Update the title
    await titleInput.clear();
    await titleInput.fill(updatedTitle);

    // Wait for debounced save to flush
    await page.waitForTimeout(3000);

    // Reload to verify persistence
    await page.reload();
    await waitForAppReady(page);

    await expect(
      page.locator('input[placeholder="UNTITLED_THREAD"]')
    ).toHaveValue(updatedTitle, { timeout: 15_000 });

    // Go back to thread list and verify the title updated there too
    await page.goto('/');
    await waitForAppReady(page);
    await expect(page.getByText(updatedTitle)).toBeVisible({ timeout: 15_000 });
    // The old title should no longer appear
    await expect(page.getByText(threadTitle, { exact: true })).not.toBeVisible();
  });

  test('should create a comment on the thread', async ({ page }) => {
    await loginUser(page, testUser);

    // Navigate to the thread via the thread list (title was updated)
    await navigateToThread(page, updatedTitle);

    const commentTextarea = page.locator('#comment-textarea');
    await expect(commentTextarea).toBeVisible({ timeout: 15_000 });

    // Type and submit comment
    await commentTextarea.fill(commentContent);
    await page.getByRole('button', { name: '[ EXECUTE_POST ]' }).click();

    // Wait for textarea to clear (indicates submission completed)
    await expect(commentTextarea).toHaveValue('', { timeout: 10_000 });

    // Wait for the comment to appear locally (local-first write)
    await expect(page.getByText(commentContent)).toBeVisible({
      timeout: 15_000,
    });

    // Reload to verify the comment was persisted to the remote DB
    await page.reload();
    await waitForAppReady(page);

    await expect(page.getByText(commentContent)).toBeVisible({
      timeout: 15_000,
    });
  });

  test('should show thread in the thread list', async ({ page }) => {
    await loginUser(page, testUser);
    await page.goto('/');
    await waitForAppReady(page);

    // Thread list should show the updated title
    await expect(page.getByText(updatedTitle)).toBeVisible({ timeout: 15_000 });

    // Click to navigate to thread detail
    await page.getByText(updatedTitle).first().click();
    await page.waitForURL(/\/thread\//, { timeout: 10_000 });

    // Verify we're on the correct thread
    await expect(
      page.locator('input[placeholder="UNTITLED_THREAD"]')
    ).toHaveValue(updatedTitle, { timeout: 15_000 });
  });
});
