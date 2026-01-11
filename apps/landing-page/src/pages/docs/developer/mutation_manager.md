---
layout: ../../../layouts/DocsLayout.astro
title: MutationManager
---

The `MutationManager` is responsible for all **Write Operations** (Create, Update, Delete) in the application. It acts as a transactional wrapper around the Local Database, ensuring that every data change is accompanied by a mutation log entry that can be synced to the cloud.

## Responsibility

- **Transactional Writes**: Wraps operations in a SurrealDB transaction (`BEGIN ... COMMIT`).
- **Mutation Logging**: Automatically creates a record in `_spooky_pending_mutations` for every data change.
- **Event Emission**: Notifies the `RouterService` (and thus `SpookySync`) immediately after a successful write.

## Architecture & Boundaries

`MutationManager` is the "Writer" in the Black Box model:

- **Inputs**: Application calls to `create`, `update`, `delete`.
- **Outputs**: `MutationCreated` events.
- **Side Effects**: Writes to Local DB.
- **Remote Access**: **NO**. It writes locally, and `SpookySync` handles the push later.

## Input/Output Reference

### Public API

| Method                    | Returns         | Description                                                 |
| :------------------------ | :-------------- | :---------------------------------------------------------- |
| `create(id, data)`        | `Promise<T>`    | Creates a record and a corresponding 'create' mutation log. |
| `update(table, id, data)` | `Promise<T>`    | Updates a record and creates an 'update' mutation log.      |
| `delete(table, id)`       | `Promise<void>` | Deletes a record and creates a 'delete' mutation log.       |

### Events (Emitted)

| Event             | Payload                                  | Description                            |
| :---------------- | :--------------------------------------- | :------------------------------------- |
| `MutationCreated` | `{ type, record_id, mutation_id, data }` | Emitted after the transaction commits. |

## Key Workflows

### The "Optimistic" Write

1. Application calls `client.create('tasks:1', { text: 'Buy milk' })`.
2. `MutationManager` constructs a SurrealQL transaction:
   ```sql
   BEGIN TRANSACTION;
   CREATE tasks:1 CONTENT { text: 'Buy milk' };
   CREATE _spooky_pending_mutations CONTENT { mutationType: 'create', ... };
   COMMIT TRANSACTION;
   ```
3. It executes this against the Local DB.
4. On success, it emits `MutationCreated`.
5. The `RouterService` immediately tells `SpookySync` to "Sync Up", pushing this change to the server in the background.

## Internal Logic

- **retry logic**: The manager includes a retry mechanism for `SQLITE_BUSY` or transaction locking errors, common in local SQLite environments.
- **Encoding**: Data is automatically encoded/decoded using Spooky's schema utilities before storage to ensure types (like Dates) are preserved.
