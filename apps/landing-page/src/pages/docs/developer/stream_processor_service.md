---
layout: ../../../layouts/DocsLayout.astro
title: StreamProcessorService
---


The `StreamProcessorService` is the engine responsible for maintaining **Materialized Views** of your data on the client. Unlike `QueryManager`, which manages the lifecycle of a query (subscriptions, heartbeats), the Stream Processor manages the _data_ itself.

## 📦 Responsibility

- **State Persistence**: Persists the state of every active query to the local database table `_spooky_stream_processor_state`.
- **Incremental Updates**: Calculates changes (diffs) to data and applies them efficiently.
- **Incantation Management**: Tracks which incantations (queries) are active and ensures they are kept in sync with the local database.

## 🔄 Workflow

1. **Registration**: When a query starts, `RouterService` calls `registerIncantation`.
2. **State Loading**: The service checks if it has existing state for this query hash.
3. **Ingestion**: As mutations happen (or initial data is loaded), data is "ingested" into the processor.
4. **Processing**: The processor runs internal circuits (DBSP) to update the view.
5. **Output**: The updated view is available for `QueryManager` to serve to the UI.

## 💾 Persistence

The service uses a local SurrealDB table `_spooky_stream_processor_state` to store:

- `hash`: The unique hash of the query/incantation.
- `state`: The serialized internal state of the processor (e.g., DBSP handle or raw data structure).

This ensures that restarting the application doesn't require re-fetching all data from scratch; the processor can resume from its last known good state.
