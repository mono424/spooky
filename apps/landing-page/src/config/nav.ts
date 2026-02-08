const basePath = process.env.ASTRO_BASE_PATH || '/spooky';
export const docsNav = [
  {
    title: 'Getting Started',
    links: [
      { text: 'Introduction', href: `${process.env.ASTRO_BASE_PATH}/docs` },
      { text: 'Installation', href: `${process.env.ASTRO_BASE_PATH}/docs/install` },
      { text: 'Schema', href: `${process.env.ASTRO_BASE_PATH}/docs/schema` },
      { text: 'Authentication', href: `${process.env.ASTRO_BASE_PATH}/docs/authentication` },
      { text: 'Query Data', href: `${process.env.ASTRO_BASE_PATH}/docs/query-data` },
      { text: 'Mutate Data', href: `${process.env.ASTRO_BASE_PATH}/docs/mutate-data` },
      { text: 'Backend Functions', href: `${process.env.ASTRO_BASE_PATH}/docs/backend-functions` },
    ],
  },
  {
    title: 'Framework Guides',
    links: [
      { text: 'SolidJS', href: `${process.env.ASTRO_BASE_PATH}/docs/guide/solid` },
      { text: 'Flutter', href: `${process.env.ASTRO_BASE_PATH}/docs/guide/flutter` },
      { text: 'Vanilla JS / TS', href: `${process.env.ASTRO_BASE_PATH}/docs/guide/vanilla` },
    ],
  },
  {
    title: 'Reference',
    links: [
      { text: 'Configuration', href: `${process.env.ASTRO_BASE_PATH}/docs/configuration` },
      { text: 'Deployment', href: `${process.env.ASTRO_BASE_PATH}/docs/deployment` },
      { text: 'Architecture', href: `${process.env.ASTRO_BASE_PATH}/docs/architecture` },
    ],
  },
];
