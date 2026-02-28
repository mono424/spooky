import { test } from '@playwright/test';
import { cleanDatabase } from '../db-cleanup';

test('clean database before test suite', async () => {
  await cleanDatabase();
});
