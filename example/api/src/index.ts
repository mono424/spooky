import dotenv from 'dotenv';

dotenv.config({ path: '.env.local' });
dotenv.config();

import { serve } from '@hono/node-server';
import app from './server';

serve({
  fetch: app.fetch,
  port: Number(process.env.PORT ?? '3000'),
});
