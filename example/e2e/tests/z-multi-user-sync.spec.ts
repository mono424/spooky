import { test, expect } from '@playwright/test';
import {
  navigateToThread,
  registerUser,
  uniqueUser,
  waitForAppReady,
  typeIntoEditor,
  waitForThreadInUpstream,
} from '../fixtures/test-fixtures';
import type { Page } from '@playwright/test';

// Cross-USER realtime sync. Each test runs with two different accounts
// (alice and bob), not two sessions of the same account — that case is
// covered by `z-multi-client-sync.spec.ts`. Cross-user is currently
// broken at the architecture level: the SSP's `update_all_edges`
// (apps/ssp/src/lib.rs ~line 2008) writes RELATE rows into
// `_00_list_ref_user_<delta.auth_id>` where `delta.auth_id` is the
// VIEW OWNER. So when alice updates a row, the fan-out only touches
// alice's per-user list_ref; bob — whose registered view depends on
// the `thread` table and SHOULD include the row per the schema's
// `published = true` permission — never receives a notification.
//
// Each test below is marked `test.fail()` so it documents the broken
// contract today. Once the SSP fan-out fix lands, removing
// `test.fail()` should turn every one of these green.

const seedDraftThread = async (
  page: Page,
  title: string,
  initialContent: string
) => {
  // The CreateThreadDialog's "Publish" button creates the thread with
  // `published: false` (see CreateThreadDialog.tsx line 71). Its
  // auto-navigation to `/thread/<id>` isn't always reliable in the
  // test environment, so we explicitly navigate via the home list
  // card after the row is visible upstream — matches the pattern in
  // z-multi-client-sync.spec.ts.
  await page.getByRole('button', { name: 'New thread' }).click();
  await page.locator('#create-title').fill(title);
  await typeIntoEditor(
    page,
    '.ProseMirror[contenteditable="true"]',
    initialContent
  );
  await page.getByRole('button', { name: 'Publish' }).click();
  await page
    .locator('#create-title')
    .waitFor({ state: 'hidden', timeout: 15_000 });
  await waitForThreadInUpstream(page, title);
  await navigateToThread(page, title);
};

// The ThreadDetail "Publish"/"Unpublish" button lives inside the
// "More options" popover. Open it, click the right label, then wait
// for the menu to close so subsequent interactions don't race.
const togglePublishFromMenu = async (
  page: Page,
  expectedLabel: 'Publish' | 'Unpublish'
) => {
  await page.getByRole('button', { name: 'More options' }).click();
  await page.getByRole('button', { name: expectedLabel, exact: true }).click();
};

const seedPublishedThread = async (
  page: Page,
  title: string,
  initialContent: string
) => {
  await seedDraftThread(page, title, initialContent);
  await togglePublishFromMenu(page, 'Publish');
  // Wait until the menu collapsed — `Unpublish` now exists in the
  // collapsed-state aria tree as soon as the next menu open. We don't
  // need to assert it directly; the test's downstream
  // waitForThreadInUpstream on the same page is the deterministic gate.
};

test.describe('Multi-user real-time sync (different accounts)', () => {
  test('alice publishing a thread makes it visible to bob', async ({ browser }) => {
    // Cross-user fan-out: alice's publish UPDATE goes through the
    // SSP's circuit, which detects the predicate transition via the
    // new `Operator::evaluate_key` re-eval pass in
    // `Circuit::step_query` and synthesizes a +1 weight in bob's
    // view's delta. The fan-out then writes a RELATE into
    // `_00_list_ref_user_<bob>` so bob's LIVE / poll fallback picks
    // it up and his home list updates.
    const alice = uniqueUser('alice');
    const bob = uniqueUser('bob');
    const title = 'Cross-User Publish Thread';

    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    try {
      await registerUser(pageA, alice);
      await registerUser(pageB, bob);

      // Bob sits on the home list watching for new threads.
      await pageB.goto('/');
      await waitForAppReady(pageB);

      // Alice creates a draft. Bob must NOT see it (schema's
      // `published = true OR author.id = $auth.id` denies bob).
      await seedDraftThread(pageA, title, 'draft body');
      await expect(
        pageB.getByText(title, { exact: true })
      ).not.toBeVisible({ timeout: 10_000 });

      // Alice publishes via the menu. The schema now permits bob to
      // read the thread, so bob's home list should pick it up.
      await togglePublishFromMenu(pageA, 'Publish');
      await waitForThreadInUpstream(pageA, title);

      await expect(
        pageB.getByText(title, { exact: true })
      ).toBeVisible({ timeout: 15_000 });
    } finally {
      await ctxA.close();
      await ctxB.close();
    }
  });

  test('title rename on a published thread propagates between users', async ({ browser }) => {
    const alice = uniqueUser('alice');
    const bob = uniqueUser('bob');
    const baseTitle = 'Cross-User Title Base';
    const renamedTitle = 'Cross-User Title Renamed';

    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    pageA.on('pageerror', (err) => console.log('[A:pageerror]', err.message));
    pageB.on('pageerror', (err) => console.log('[B:pageerror]', err.message));

    try {
      await registerUser(pageA, alice);
      await registerUser(pageB, bob);

      await seedPublishedThread(pageA, baseTitle, 'base body');

      // Bob's home list should already see the published thread (this
      // is the "initial snapshot" path — register_view_handler seeds
      // bob's per-user list_ref at registration, which DOES work
      // cross-user). If this assertion times out, bug is in the
      // initial-snapshot fan-out, not the live-update fan-out.
      await pageB.goto('/');
      await waitForAppReady(pageB);
      await expect(
        pageB.getByText(baseTitle, { exact: true })
      ).toBeVisible({ timeout: 15_000 });

      // Alice renames via the detail-page input.
      const titleInputA = pageA.locator('input[placeholder="Untitled"]');
      await expect(titleInputA).toBeVisible({ timeout: 10_000 });
      await titleInputA.fill(renamedTitle);
      await waitForThreadInUpstream(pageA, renamedTitle);

      await expect(
        pageB.getByText(renamedTitle, { exact: true })
      ).toBeVisible({ timeout: 15_000 });
      await expect(
        pageB.getByText(baseTitle, { exact: true })
      ).not.toBeVisible();
    } finally {
      await ctxA.close();
      await ctxB.close();
    }
  });

  test('unpublishing a thread removes it from the other user’s home list', async ({ browser }) => {
    // Inverse of test 1: when alice toggles published back to false,
    // the same evaluate_key pass detects that bob's view's predicate
    // now rejects the row and synthesizes a negative weight that
    // cancels the cache entry; `update_all_edges` issues a DELETE on
    // bob's `_00_list_ref_user_<bob>` edge.
    const alice = uniqueUser('alice');
    const bob = uniqueUser('bob');
    const title = 'Cross-User Unpublish Thread';

    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    try {
      await registerUser(pageA, alice);
      await registerUser(pageB, bob);

      await seedPublishedThread(pageA, title, 'will be unpublished');

      await pageB.goto('/');
      await waitForAppReady(pageB);
      await expect(
        pageB.getByText(title, { exact: true })
      ).toBeVisible({ timeout: 15_000 });

      // Alice unpublishes via the menu.
      await togglePublishFromMenu(pageA, 'Unpublish');

      // Bob's home list must drop the title within the realtime
      // window. Poll-with-timeout via the negated visibility check.
      await expect(
        pageB.getByText(title, { exact: true })
      ).not.toBeVisible({ timeout: 15_000 });
    } finally {
      await ctxA.close();
      await ctxB.close();
    }
  });

  test('CRDT body edits propagate from alice to bob on a published thread', async ({ browser }) => {
    // INCORRECT TEST PREMISE — left as `test.fail()` until rewritten.
    // The schema (example/schema/src/schema.surql ~thread permissions)
    // allows UPDATE on a thread only for the author OR users in the
    // `collaborates_on` relation. `published = true` grants SELECT but
    // NOT UPDATE. ThreadDetail.tsx::canEdit reflects this — bob's
    // editor is rendered with `contenteditable="false"`, so the
    // `.ProseMirror[contenteditable="true"]` selector doesn't match.
    // To validate cross-user CRDT propagation this test should either
    // (a) add bob via `collaborates_on` before alice types, or
    // (b) check for the read-only rendered body text instead of the
    // editor. The list_ref-driven `applyRow` hook in sp00ky.ts already
    // forwards parent-row updates into CrdtManager, so collaborator
    // propagation is wired regardless.
    test.fail();
    const alice = uniqueUser('alice');
    const bob = uniqueUser('bob');
    const title = 'Cross-User CRDT Thread';
    const seed = 'shared seed';
    const appended = ' — alice typed';

    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    try {
      await registerUser(pageA, alice);
      await registerUser(pageB, bob);

      await seedPublishedThread(pageA, title, seed);

      // Both navigate to the thread detail page so both LoroSyncPlugin
      // instances are mounted before the first keystroke. Without this,
      // bob's CRDT subscription might miss the initial ops.
      const aliceThreadHref = await pageA.url();
      // Bob navigates via the home list to be a realistic end-user
      // path; he must first see the published thread there.
      await pageB.goto('/');
      await waitForAppReady(pageB);
      await expect(
        pageB.getByText(title, { exact: true })
      ).toBeVisible({ timeout: 15_000 });
      await pageB.goto(aliceThreadHref);
      await waitForAppReady(pageB);

      const editorA = pageA
        .locator('.ProseMirror[contenteditable="true"]')
        .first();
      const editorB = pageB
        .locator('.ProseMirror[contenteditable="true"]')
        .first();
      await expect(editorA).toBeVisible({ timeout: 15_000 });
      await expect(editorB).toBeVisible({ timeout: 15_000 });
      await expect(editorA).toContainText(seed, { timeout: 15_000 });
      await expect(editorB).toContainText(seed, { timeout: 15_000 });

      await editorA.click();
      await pageA.keyboard.press('End');
      await pageA.keyboard.type(appended);

      await expect(editorB).toContainText(appended, { timeout: 30_000 });
    } finally {
      await ctxA.close();
      await ctxB.close();
    }
  });

  test('alice’s cursor and username label are visible to bob on a published thread', async ({ browser }) => {
    // INCORRECT TEST PREMISE — same as the CRDT-body test above. Bob
    // can SELECT a published thread but not UPDATE it, so the test's
    // `.ProseMirror[contenteditable="true"]` selector matches nothing
    // on bob's side. To validate cursor-label visibility cross-user
    // the test must (a) add bob to `collaborates_on` first, or (b)
    // assert against the read-only rendering of remote peers.
    test.fail();
    const alice = uniqueUser('alice');
    const bob = uniqueUser('bob');
    const title = 'Cross-User Cursor Thread';

    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    try {
      await registerUser(pageA, alice);
      await registerUser(pageB, bob);

      await seedPublishedThread(pageA, title, 'cursor seed');
      const aliceThreadHref = pageA.url();

      // Bob loads the thread directly. Going through the home list
      // first would conflate visibility (test 1) with presence; we
      // already know visibility is broken, so we navigate directly
      // by URL to isolate the cursor-presence contract.
      await pageB.goto(aliceThreadHref);
      await waitForAppReady(pageB);

      const editorA = pageA
        .locator('.ProseMirror[contenteditable="true"]')
        .first();
      const editorB = pageB
        .locator('.ProseMirror[contenteditable="true"]')
        .first();
      await expect(editorA).toBeVisible({ timeout: 15_000 });
      await expect(editorB).toBeVisible({ timeout: 15_000 });

      // Alice places her caret and does a no-op edit (the
      // CollaborativeEditor's presence push is debounced 100ms; a
      // tiny edit forces a cursor-move event to fire reliably).
      await editorA.click();
      await pageA.keyboard.press('End');
      await pageA.keyboard.type('.');
      await pageA.keyboard.press('Backspace');

      // The LoroSyncPlugin renders remote peers as
      //   <span class="ProseMirror-loro-cursor" style="border-color:..">
      //     <div style="background-color:..">username</div>
      //   </span>
      // Bob sees alice's cursor labelled with alice's username.
      const remoteCursor = pageB.locator('.ProseMirror-loro-cursor');
      await expect(remoteCursor.first()).toBeVisible({ timeout: 30_000 });
      await expect(remoteCursor.locator('div').first()).toContainText(
        alice.username,
        { timeout: 10_000 }
      );
    } finally {
      await ctxA.close();
      await ctxB.close();
    }
  });
});
