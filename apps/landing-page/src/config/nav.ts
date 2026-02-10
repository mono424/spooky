export const basePath = process.env.ASTRO_BASE_PATH || '/spooky';
export const docsNav = [
  {
    title: 'Getting Started',
    links: [
      { text: 'Introduction', href: `${basePath}/docs` },
      { text: 'Installation', href: `${basePath}/docs/install` },
      { text: 'Schema', href: `${basePath}/docs/schema` },
      { text: 'Authentication', href: `${basePath}/docs/authentication` },
      { text: 'Query Data', href: `${basePath}/docs/query-data` },
      { text: 'Mutate Data', href: `${basePath}/docs/mutate-data` },
      { text: 'Backend Functions', href: `${basePath}/docs/backend-functions` },
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
    title: 'Reference',
    links: [
      { text: 'Configuration', href: `${basePath}/docs/configuration` },
      { text: 'Deployment', href: `${basePath}/docs/deployment` },
      { text: 'Architecture', href: `${basePath}/docs/architecture` },
      { text: 'Scheduler API', href: `${basePath}/docs/scheduler-api` },
      { text: 'SSP API', href: `${basePath}/docs/ssp-api` },
    ],
  },
];
