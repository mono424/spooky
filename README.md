<div align="center">

# Spooky 👻

**The Reactive, Local-First Framework for SurrealDB**

> **⚠️ Under active development — not production-ready. APIs may change without notice.**

[![npm version](https://img.shields.io/npm/v/@spooky-sync/core?label=%40spooky-sync%2Fcore&color=cb3837)](https://www.npmjs.com/package/@spooky-sync/core)
[![npm downloads](https://img.shields.io/npm/dm/@spooky-sync/core)](https://www.npmjs.com/package/@spooky-sync/core)
[![bundle size](https://img.shields.io/bundlejs/size/@spooky-sync/core)](https://bundlejs.com/?q=%40spooky-sync%2Fcore)
[![npm version](https://img.shields.io/npm/v/@spooky-sync/cli?label=%40spooky-sync%2Fcli&color=cb3837)](https://www.npmjs.com/package/@spooky-sync/cli)
[![license](https://img.shields.io/github/license/mono424/spooky)](https://github.com/mono424/spooky)
[![stars](https://img.shields.io/github/stars/mono424/spooky)](https://github.com/mono424/spooky/stargazers)
[![last commit](https://img.shields.io/github/last-commit/mono424/spooky)](https://github.com/mono424/spooky/commits/main)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![SurrealDB](https://img.shields.io/badge/SurrealDB-FF00A0?logo=surrealdb&logoColor=white)](https://surrealdb.com/)

[Documentation](https://mono424.github.io/spooky/) · [Example App](example/app-solid) · [CLI](https://www.npmjs.com/package/@spooky-sync/cli) · [Contributing](#contributing)

</div>

## Features

- **Live Queries** — Your UI updates instantly when data changes
- **Local-First** — Works offline using IndexedDB, syncs when back online
- **End-to-End Type Safety** — Generated TypeScript definitions from your SQL schema
- **Optimistic UI** — Immediate feedback for user actions while syncing in the background

## Quick Start

### Install

```bash
pnpm add @spooky-sync/client-solid
```

### Generate Types with CLI

```bash
npx @spooky-sync/cli generate
```

### Usage (SolidJS)

```tsx
import { useQuery } from '@spooky-sync/client-solid';
import { db } from './db';

const ThreadList = () => {
  const threads = useQuery(() => db.query('thread').select('*').all());

  return (
    <ul>
      <For each={threads.data}>{(thread) => <li>{thread.title}</li>}</For>
    </ul>
  );
};
```

## Packages

| Package | Description |
|---------|-------------|
| [`@spooky-sync/core`](https://www.npmjs.com/package/@spooky-sync/core) | Core client SDK — sync engine, caching, reactivity |
| [`@spooky-sync/client-solid`](https://www.npmjs.com/package/@spooky-sync/client-solid) | SolidJS bindings (`useQuery`, etc.) |
| [`@spooky-sync/query-builder`](https://www.npmjs.com/package/@spooky-sync/query-builder) | Type-safe query builder |
| [`@spooky-sync/cli`](https://www.npmjs.com/package/@spooky-sync/cli) | CLI for schema generation |

## Example App

Check out the full-featured reference app built with SolidJS:

```bash
cd example/app-solid && pnpm install && pnpm dev
```

## Documentation

Full documentation is available at **[mono424.github.io/spooky](https://mono424.github.io/spooky/)**.

## Contributing

Contributions are welcome! This is a monorepo — see the individual package directories under `packages/` for details.

## License

[ISC](LICENSE)

---

<div align="center">
If you find Spooky useful, consider giving it a ⭐ on <a href="https://github.com/mono424/spooky">GitHub</a>!
</div>
