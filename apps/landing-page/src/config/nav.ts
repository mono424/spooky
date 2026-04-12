export const basePath = process.env.ASTRO_BASE_PATH || '';
export const docsNav = [
  {
    title: 'Getting Started',
    links: [
      { text: 'Introduction', href: `${basePath}/docs` },
      { text: 'Installation', href: `${basePath}/docs/install` },
      { text: 'Schema', href: `${basePath}/docs/schema` },
      { text: 'Authentication', href: `${basePath}/docs/authentication` },
      { text: 'Environment Variables', href: `${basePath}/docs/environment-variables` },
    ],
  },
  {
    title: 'Data',
    links: [
      { text: 'Query Data', href: `${basePath}/docs/query-data` },
      { text: 'Mutate Data', href: `${basePath}/docs/mutate-data` },
      { text: 'Migrations', href: `${basePath}/docs/migrations` },
      { text: 'File Buckets', href: `${basePath}/docs/buckets` },
    ],
  },
  {
    title: 'Backend',
    links: [
      { text: 'Setup', href: `${basePath}/docs/backend/setup` },
      { text: 'Jobs & Usage', href: `${basePath}/docs/backend/jobs` },
      { text: 'Dev Server', href: `${basePath}/docs/backend/dev-server` },
    ],
  },
  {
    title: 'Framework Guides',
    links: [
      { text: 'SolidJS', href: `${basePath}/docs/guide/solid` },
      { text: 'Flutter', href: `${basePath}/docs/guide/flutter` },
      { text: 'Vanilla JS / TS', href: `${basePath}/docs/guide/vanilla` },
    ],
  },
  {
    title: 'Cloud',
    links: [
      { text: 'Getting Started', href: `${basePath}/docs/cloud/getting-started` },
      { text: 'Deploying', href: `${basePath}/docs/cloud/deploying` },
      { text: 'Logs & Monitoring', href: `${basePath}/docs/cloud/logs` },
      { text: 'CI/CD', href: `${basePath}/docs/cloud/ci-cd` },
      { text: 'Environment Variables', href: `${basePath}/docs/cloud/env-variables` },
      { text: 'Teams', href: `${basePath}/docs/cloud/team` },
      { text: 'Backups', href: `${basePath}/docs/cloud/backups` },
      { text: 'Vault Architecture', href: `${basePath}/docs/cloud/vault` },
    ],
  },
  {
    title: 'Reference',
    links: [
      { text: 'Configuration', href: `${basePath}/docs/configuration` },
      { text: 'Self-Hosted Deployment', href: `${basePath}/docs/deployment` },
      { text: 'Architecture', href: `${basePath}/docs/architecture` },
      { text: 'Scheduler API', href: `${basePath}/docs/scheduler-api` },
      { text: 'SSP API', href: `${basePath}/docs/ssp-api` },
    ],
  },
];
