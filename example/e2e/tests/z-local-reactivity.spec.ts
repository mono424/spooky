import {
  test,
  expect,
  registerUser,
  testUser,
  waitForAppReady,
  typeIntoEditor,
  waitForThreadInUpstream,
} from '../fixtures/test-fixtures';

// Single-client query-wide reactivity. The thread detail page renders a
// `<ThreadSidebar>` (the `<aside>`) that runs its own `db.query('thread')`
// query, separate from the detail's `query('thread').where({id})`. Editing
// the title in the detail page should propagate to the sidebar and the
// list view via the SSP's incremental dataflow — no reload, no manual
// refetch.

test.describe.serial('Local query-wide reactivity', () => {
  // Distinct titles so this file's threads don't collide with other
  // specs that share the DB across the run.
  const baseTitle = 'Reactivity Local Thread';
  const renamedTitle = 'Reactivity Local Thread (renamed)';

  test('sidebar and list reflect title edits without reload', async ({ page }) => {
    // Verifies that local `useQuery` subscriptions react to UPDATEs on
    // rows containing `TYPE bytes` CRDT columns. Previously broke
    // because `StreamProcessorService.normalizeValue` passed
    // `Uint8Array` directly into the wasm boundary and
    // `serde_wasm_bindgen::from_value` rejected it. Fixed by stripping
    // bytes to `null` at normalization time (SSP can't filter on opaque
    // bytes anyway).
    await registerUser(page, testUser);

    await page.getByRole('button', { name: 'New thread' }).click();
    await page.locator('#create-title').fill(baseTitle);
    await typeIntoEditor(page, '.ProseMirror[contenteditable="true"]', 'reactive content');
    await page.getByRole('button', { name: 'Publish' }).click();
    await page
      .locator('#create-title')
      .waitFor({ state: 'hidden', timeout: 15_000 });

    // Gate the rest of the test on the row showing up upstream — the
    // create flow's sync-up is debounced and the home list query lags
    // through DBSP+SSP+sync-down, so a fast `goto('/')` can race
    // propagation and not see the new thread.
    await waitForThreadInUpstream(page, baseTitle);

    await page.goto('/');
    await waitForAppReady(page);
    await expect(page.getByText(baseTitle, { exact: true })).toBeVisible({
      timeout: 15_000,
    });

    const card = page
      .locator('[data-thread-index]')
      .filter({ has: page.getByText(baseTitle, { exact: true }) });
    await card.first().click();
    await page.waitForURL(/\/thread\//, { timeout: 10_000 });

    // Sidebar lives at <aside>. The same `useQuery(...thread...)` that
    // feeds it is independent from the detail's thread-by-id query, so
    // it's a clean check of cross-query reactivity.
    const sidebar = page.locator('aside');
    await expect(
      sidebar.getByText(baseTitle, { exact: true })
    ).toBeVisible({ timeout: 15_000 });

    const titleInput = page.locator('input[placeholder="Untitled"]');
    await expect(titleInput).toBeVisible({ timeout: 10_000 });
    await titleInput.fill(renamedTitle);

    // Confirm the rename actually reached upstream — without this gate
    // a sidebar-not-updating failure could be either "title save broke"
    // or "query doesn't react to local updates" and we couldn't tell.
    await waitForThreadInUpstream(page, renamedTitle);

    // Title saves are debounced (`db.update(..., { debounced: true })`,
    // ~200ms). 15s gives the SSP push round-trip enough margin without
    // dragging the suite when the assertion holds.
    await expect(
      sidebar.getByText(renamedTitle, { exact: true })
    ).toBeVisible({ timeout: 15_000 });
    await expect(
      sidebar.getByText(baseTitle, { exact: true })
    ).not.toBeVisible();

    // List view is also a separate `db.query('thread')` query — verify
    // the same reactive update reaches it without a reload.
    await page.goto('/');
    await waitForAppReady(page);
    await expect(page.getByText(renamedTitle, { exact: true })).toBeVisible({
      timeout: 15_000,
    });
    await expect(
      page.getByText(baseTitle, { exact: true })
    ).not.toBeVisible();
  });
});
