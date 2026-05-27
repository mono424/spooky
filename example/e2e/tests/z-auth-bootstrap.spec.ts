import { test, expect, uniqueUser, registerUser } from '../fixtures/test-fixtures';

// Regression guard for #41: on a clean signup, the SSP's
// `ingest_handler` should pre-emptively create the new user's
// `_00_list_ref_user_<id>` table when the user CREATE event arrives.
// Without that, `Sp00kySync::setCurrentUserId` races the table's lazy
// creation in `register_view_handler` and the initial
// `LIVE SELECT * FROM _00_list_ref_user_<id>` fails with
// "table not found", retried with a backoff that's surfaced via
// `Sp00kyClient.liveRetryCount`. If that counter is non-zero on a
// fresh register, the pre-emptive path regressed.
test('clean signup never retries the initial list_ref LIVE registration', async ({ page }) => {
  const user = uniqueUser('bootstrap');
  await registerUser(page, user);

  // `setCurrentUserId` runs synchronously from `auth.subscribe` after
  // sign-in; by the time the authenticated layout (`New thread`) is
  // visible the LIVE attempt loop has either resolved or exhausted
  // its retries. We give it a small grace window so the test isn't
  // sensitive to the exact ordering of solid signals.
  await page.waitForFunction(
    () => (window as unknown as { __sp00ky__?: unknown }).__sp00ky__ !== undefined,
    { timeout: 5_000 }
  );

  const liveRetryCount = await page.evaluate(() => {
    const w = window as unknown as { __sp00ky__?: { liveRetryCount: number } };
    return w.__sp00ky__?.liveRetryCount ?? -1;
  });

  expect(liveRetryCount).toBe(0);
});
