export type FeatureGroup = 'core' | 'cloud';

export interface Feature {
  slug: string;
  iconKey: string;
  title: string;
  desc: string;
  longDesc: string;
  group: FeatureGroup;
  href: string;
}

export const coreFeatures: Feature[] = [
  {
    slug: 'optimistic-updates',
    iconKey: 'optimistic',
    title: 'Optimistic Updates',
    desc: 'UI updates instantly, sync happens in the background.',
    longDesc:
      'Writes land on-device in milliseconds. sp00ky reconciles with the server in the background, so your UI never waits on a round-trip and never feels sluggish — even over a flaky connection.',
    group: 'core',
    href: '/features/optimistic-updates',
  },
  {
    slug: 'reactive-queries',
    iconKey: 'query',
    title: 'Reactive Queries',
    desc: 'SurrealQL queries that update when data changes.',
    longDesc:
      'Subscribe to a SurrealQL query once — sp00ky pushes fresh results whenever the underlying data changes, anywhere in the system. No polling, no manual invalidation, no stale state.',
    group: 'core',
    href: '/features/reactive-queries',
  },
  {
    slug: 'job-scheduler',
    iconKey: 'scheduler',
    title: 'Job Scheduler',
    desc: 'Retries, queues, durable cross-session tasks.',
    longDesc:
      'A durable, local-first job scheduler built into the engine. Retries, backoff, queues, and cross-session persistence — without standing up a separate worker or queue service.',
    group: 'core',
    href: '/features/job-scheduler',
  },
  {
    slug: 'file-sync',
    iconKey: 'files',
    title: 'File Sync',
    desc: 'Binary assets replicated alongside your data.',
    longDesc:
      'Attach files to your records and sp00ky replicates them alongside your data. Chunked transfers, resumable uploads, and offline-first caching come out of the box.',
    group: 'core',
    href: '/features/file-sync',
  },
  {
    slug: 'type-safety',
    iconKey: 'types',
    title: 'End-to-end Type Safety',
    desc: 'Schema-first codegen. Types flow from database to UI.',
    longDesc:
      'Define your schema in SurrealQL once. sp00ky generates matching types for every client language, so autocomplete and compile-time checks cover the whole stack. No hand-written DTOs, no stale types, no drift between server and client.',
    group: 'core',
    href: '/features/type-safety',
  },
  {
    slug: 'devtools',
    iconKey: 'devtools',
    title: 'First-class Tooling',
    desc: 'Chrome DevTools, an MCP server, and a CLI. Debug in real time.',
    longDesc:
      'sp00ky ships with a Chrome extension that inspects every query and mutation, an MCP server so your agent can drive sp00ky apps directly, and a CLI that scaffolds, deploys, and migrates. Everything you would build anyway, already built.',
    group: 'core',
    href: '/features/devtools',
  },
];

export const cloudFeatures: Feature[] = [
  {
    slug: 'shared-vault',
    iconKey: 'vault',
    title: 'Shared Vault',
    desc: 'End-to-end encrypted shared stores across devices.',
    longDesc:
      'Securely share encrypted stores between users and devices. Vault keys are managed per-workspace, so teammates can collaborate on the same dataset without ever exposing plaintext to the cloud.',
    group: 'cloud',
    href: '/cloud/shared-vault',
  },
  {
    slug: 'automatic-backups',
    iconKey: 'backup',
    title: 'Automatic Backups',
    desc: 'Continuous point-in-time snapshots, 1-click restore.',
    longDesc:
      'Continuous snapshots run in the background. Roll back to any point in time with a single click — no tickets, no pipelines, no downtime.',
    group: 'cloud',
    href: '/cloud/automatic-backups',
  },
  {
    slug: 'managed-hosting',
    iconKey: 'hosting',
    title: 'Managed Hosting',
    desc: 'Spin up production-ready instances in one command.',
    longDesc:
      'Production-ready sp00ky instances, spun up with a single command. Autoscaling, TLS, logs, and metrics are managed for you — focus on the app, not the infra.',
    group: 'cloud',
    href: '/cloud/managed-hosting',
  },
];

export const allFeatures: Feature[] = [...coreFeatures, ...cloudFeatures];

export function findFeature(slug: string): Feature | undefined {
  return allFeatures.find((f) => f.slug === slug);
}
