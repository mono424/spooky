---
layout: ../../../layouts/DocsLayout.astro
title: RouterService
---

# RouterService

The `RouterService` is the **Central Nervous System** of the Spooky Core. In a strictly decoupled architecture, services do not speak to each other directly. Instead, they emit events to the Router, which forwards them to the appropriate destination based on a deterministic routing table.

## 📦 Responsibility

- **Event Orchestration**: Listens to everything, routes everything.
- **Decoupling**: Ensures `MutationManager` doesn't need to import `SpookySync`.
- **Traffic Control**: Provides a single place to see how data flows through the system.

## 🏗️ Architecture & Boundaries

The Router sits in the middle of the Core module:

- **Inputs**: Events from ALL other services.
- **Outputs**: Method calls to ALL other services.
- **State**: Stateless.

## 🛣️ Routing Table

The following table defines the "Constitution" of the Spooky Core application.

| Source       | Event                         | Target            | Action                 | Reason                                                  |
| :----------- | :---------------------------- | :---------------- | :--------------------- | :------------------------------------------------------ |
| **Mutation** | `MutationCreated`             | `DevTools`        | `onMutation`           | Log the user's action for debugging.                    |
| **Mutation** | `MutationCreated`             | `Sync`            | `refreshIncantations`  | Local data changed; we must re-evaluate active queries. |
| **Query**    | `IncantationInitialized`      | `DevTools`        | `onQueryInit`          | Log that a new query started.                           |
| **Query**    | `IncantationInitialized`      | `Sync`            | `enqueueDownEvent`     | Tell remote to register this client's interest.         |
| **Query**    | `IncantationInitialized`      | `StreamProcessor` | `registerIncantation`  | Register query for local stream processing.             |
| **Query**    | `IncantationRemoteHashUpdate` | `Sync`            | `enqueueDownEvent`     | Local tree differs from remote; trigger a sync.         |
| **Query**    | `IncantationTTLHeartbeat`     | `Sync`            | `enqueueDownEvent`     | Keep the query alive on the server.                     |
| **Query**    | `IncantationCleanup`          | `Sync`            | `enqueueDownEvent`     | Unsubscribe/cleanup on the server.                      |
| **Query**    | `IncantationUpdated`          | `DevTools`        | `onQueryUpdated`       | Log that the UI received new data.                      |
| **Sync**     | `IncantationUpdated`          | `Query`           | `handleIncomingUpdate` | Sync fetched new data; push it to the UI layer.         |

## 🔑 Key Workflows

### The "Reactive Loop"

1. User writes data -> `MutationManager` -> **Router**
2. **Router** -> `SpookySync` (Sync Up)
3. `SpookySync` -> Remote DB
4. Remote DB updates -> Other Clients
5. ... Meanwhile ...
6. **Router** -> `QueryManager` (Refresh)
7. `QueryManager` -> UI (Reflects write immediately)
