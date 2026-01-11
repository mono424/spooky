---
layout: ../../../layouts/DocsLayout.astro
title: DevToolsService
---

The `DevToolsService` is a specialized service designed to expose the internal state of the Spooky framework to external debugging tools, specifically the **Spooky Chrome Extension**. It acts as a passive observer, receiving copies of events via the Router and broadcasting them to the browser's window object or extension port.

## Responsibility

- **State Exposure**: Makes the internal logs, query states, and mutation history visible.
- **Bridge**: Acts as the bridge between the Core module (in-memory) and the DevTools UI (extension).

## Architecture & Boundaries

- **Service Level**: Tooling / Instrumentation
- **Inputs**: Method calls triggered by the `RouterService` (e.g., `onMutation`, `onQueryUpdated`).
- **Outputs**: `window.postMessage` or specific custom events for the extension.
- **Dependencies**: `LocalDatabaseService` (read-only access to inspect tables).

## Input/Output Reference

### Methods (Called by Router)

| Method                        | Description                                                 |
| :---------------------------- | :---------------------------------------------------------- |
| `onQueryInitialized(payload)` | Reports that a new query has been registered.               |
| `onQueryUpdated(payload)`     | Reports data updates (including record counts and latency). |
| `onMutation(payload)`         | Reports a write operation.                                  |

## Key Workflows

### Debugging a Query

1. Developer opens Spooky DevTools.
2. The UI queries a list of items.
3. `RouterService` detects `IncantationInitialized` and calls `DevTools.onQueryInitialized`.
4. `DevToolsService` formats the message and sends it to the extension.
5. The extension displays the active query, its SQL, and its parameters.
6. When data arrives, `RouterService` calls `DevTools.onQueryUpdated`.
7. The extension highlights the query and updates the record count.

## Internal Logic

- **Performance**: The service is designed to be lightweight. It does minimal processing.
- **Safety**: In production builds, this service can be disabled or stubbed out to prevent state leakage.
