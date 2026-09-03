# Sp00ky Admin Dashboard

The operator console a scheduler serves at `/admin`. Solid.js, no runtime
dependencies beyond `solid-js` and `@solidjs/router`.

Full documentation: [Admin dashboard](https://sp00ky.dev/docs/reference/admin-dashboard).

## What it shows

- **Overview** — scheduler and SSP health, backend health, ingest lag, and the
  end-to-end sync latency with its sparkline
- **SSPs** — per-processor state, with a bootstrap progress bar
- **Backends** — health status and a ~30-minute response-time history
- **Workflows** — runs updating live over SSE, with per-step detail
- **Schedules** — definitions with next and last fire
- **Logs** — live tail of scheduler or SSP output

## Development

```bash
pnpm install
pnpm --filter @spooky-sync/dashboard dev     # http://localhost:4300
```

The dev server is not served by a scheduler, so the app comes up in
**standalone** mode: the sign-in form shows an endpoint field. Point it at a
running scheduler's admin port (`http://localhost:9668`).

To run it the way it actually ships:

```bash
pnpm --filter @spooky-sync/dashboard build
SPKY_ADMIN_DIR=apps/dashboard/dist cargo run -p scheduler
# http://localhost:9668/admin
```

## Shape of the code

| Path | Role |
| --- | --- |
| `src/api/client.ts` | Endpoint resolution, bearer token, `fetch`-based SSE reader |
| `src/api/types.ts` | Wire types for `/admin/api` |
| `src/lib/format.ts` | Value formatters |
| `src/lib/status.ts` | Status → colour, in one place |
| `src/components/Chrome.tsx` | Page head, panels, metric rail, pills, key/value |
| `src/components/Timeline.tsx` | The run Gantt — the workflow page's centrepiece |
| `src/components/Sparkline.tsx` | Latency/response-time chart |
| `src/components/` | Shell, bootstrap progress |
| `src/routes/` | One file per view |
| `src/styles/theme.css` | Design tokens and every rule |

## Conventions worth keeping

- **Green and red are reserved for status.** Nothing decorative uses them, which
  is what keeps a red dot meaning something is wrong. Go through
  `lib/status.ts` rather than picking a class inline.
- **No charting library.** `components/Sparkline.tsx` is hand-rolled SVG. A
  failed probe is drawn as a *gap*, not a full-height bar — an outage is the
  absence of a measurement, not the slowest possible one.
- **No webfonts.** The dashboard is served by a scheduler on a private network,
  so a request to a font CDN would hang. The character comes from the
  monospace-forward hierarchy, not a downloaded typeface.
- **The timeline must not draw work that did not happen.** `_00_step_run`
  defaults `created_at` at row creation, so a `blocked` step has a timestamp;
  plotting from it invents a bar. See `hasStarted` in `WorkflowDetail.tsx`.
- **`type::string(NONE)` is the string `"NONE"`, not null.** Any nullable column
  a query stringifies — timestamps and record links alike — must go through
  `isAbsent` / `orNull` in `lib/format.ts`, or an unset link becomes a dead
  "open →".
- **One poll for the whole app.** `App.tsx` polls `/overview` and passes it
  down. Views should not add their own poll for data that is already there.
- **`EventSource` is unusable here** — it cannot send an `Authorization`
  header, and we deliberately do not use cookies. `openStream()` in
  `api/client.ts` reads the `fetch` body and parses the event stream by hand.
- **`formatRelativeTime`, `formatDuration`, `formatMs` and `formatBytes` are
  duplicated** from `apps/devtools/src/utils/formatters.ts`. That app is a
  Chrome extension with no package exports, so a workspace import is not
  available; keep the two in step by hand.
