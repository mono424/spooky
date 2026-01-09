---
layout: ../../../layouts/DocsLayout.astro
title: SpookySync Service
---

# SpookySync

`SpookySync` is the synchronization engine of the application. It acts as the bridge between the **Local Database** (in-browser) and the **Remote Database** (cloud). It is the only service permitted to read or write to the remote backend, ensuring a single point of control for data consistency.

## 📦 Responsibility

- **Two-Way Sync**: Pushes local changes Up (`syncUp`) and pulls remote changes Down (`syncDown`).
- **Live Queries**: Maintains active listeners on the remote database for real-time updates.
- **Orphan Management**: Verifies and purges "ghost records" that exist locally but have been deleted remotely.

## 🏗️ Architecture & Boundaries

`SpookySync` is a "privileged" service in the Black Box model:

- **Inputs**:
  - `MutationEnqueued` events (via `RouterService`) to trigger Up Sync.
  - `Incantation` events (via `RouterService`) to manage query subscriptions.
- **Outputs**:
  - `IncantationUpdated` events (routed to `QueryManager`) when new data arrives.
- **Remote Access**: **YES**. This is the exclusive owner of the remote connection for data operations.

## 🔄 Input/Output Reference

### Methods

| Method                         | Description                                                              |
| :----------------------------- | :----------------------------------------------------------------------- |
| `init()`                       | Initializes the Up/Down queues and starts the Live Query listener.       |
| `enqueueDownEvent(event)`      | Receives commands (Register, Sync, Heartbeat) from the Router.           |
| `enqueueMutation(mutations)`   | Receives local mutations from the Router to queue for Up Sync.           |
| `refreshIncantations(queries)` | Forces a re-fetch for specific queries (usually after a local mutation). |

### Events (Emitted)

| Event                | Payload                      | Description                                                    |
| :------------------- | :--------------------------- | :------------------------------------------------------------- |
| `IncantationUpdated` | `{ incantationId, records }` | Fired when new data is successfully synced and cached locally. |

## 🔑 Key Workflows

### 1. Live Query Flow (The "Incantation" Loop)

1. **Startup**: `SpookySync` subscribes to `LIVE SELECT * FROM _spooky_incantation`.
2. **Update**: When the remote server updates an Incantation (e.g., due to a data change from another client), `SpookySync` receives a notification.
3. **Fetch**: The service calculates the difference between the local Merkle Tree and the remote Merkle Tree.
4. **Sync**: It fetches only the missing/changed records (`delta sync`).
5. **Cache**: Records are saved to the Local DB.
6. **Notify**: An `IncantationUpdated` event is emitted, which the `RouterService` forwards to `QueryManager` to update the UI.

### 2. Orphan Verification

Occasionally, a record might be deleted remotely but missed by a delta update.

1. `SpookySync` compares the IDs in the current local result set against the remote Merkle Tree.
2. If a local ID is **not** present in the remote tree, it is flagged as a potential orphan.
3. The service verifies the deletion against the remote DB and purges the ghost record if confirmed.

## 🧠 Internal Logic: Merkle Trees

Spooky uses **Merkle Trees** (or Hash Trees) to efficiently verify data integrity. Instead of comparing every record, we compare hashes of the data. If the root hashes match, the data is identical. If they differ, we traverse the tree to find exactly which leaf nodes (records) changed, minimizing network bandwidth.
