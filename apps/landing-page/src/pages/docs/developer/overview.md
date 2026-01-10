---
layout: ../../../layouts/DocsLayout.astro
title: System Overview
---

Spooky's Core module is built on a **Strict "Black Box" Service Architecture**, designed to ensure modularity, predictable data flow, and clearly defined boundaries between local and remote operations.

import Icon from '../../../components/ui/Icon.astro';

## <Icon name="brain" /> The "Black Box" Pattern

Each service in the Core module operates as an independent unit with:

- **Strictly Defined Inputs**: Public methods and specific events they consume.
- **Strictly Defined Outputs**: Emitted events and return values.
- **Minimal Dependencies**: Services do not know about each other's internal state.

The **RouterService** acts as the central nervous system, orchestrating communication between these black boxes. This prevents "spaghetti code" where services are tightly coupled.

## <Icon name="building" /> Architecture Diagram

The following diagram illustrates the high-level data flow and service relationships:

```plaintext
graph TD
    subgraph Client ["Client Application"]
        UI[User Interface]
        QM[QueryManager]
        MM[MutationManager]
    end

    subgraph Core ["Spooky Core"]
        Router[RouterService]
        Sync[SpookySync]
        Auth[AuthService]
        LocalDB[(Local Database)]
    end

    subgraph Cloud ["Spooky Cloud"]
        RemoteDB[(Remote Database)]
    end

    %% Data Flow
    UI -->|Read Data| QM
    UI -->|Write Data| MM

    QM -->|Read| LocalDB
    MM -->|Write| LocalDB

    MM -->|MutationCreated| Router
    Router -->|Notify| Sync

    Sync -->|Sync Up/Down| RemoteDB
    Sync -->|Update| LocalDB
    Sync -->|IncantationUpdated| Router

    Router -->|Notify| QM
    QM -->|Update UI| UI

    Router -->|Register| StreamProc[StreamProcessor]
    StreamProc -->|Read/Write| LocalDB

    Auth -->|Authenticate| RemoteDB

    style Router fill:#f9f,stroke:#333,stroke-width:2px
    style Sync fill:#ccf,stroke:#333,stroke-width:2px
    style LocalDB fill:#dfd,stroke:#333,stroke-width:2px
    style RemoteDB fill:#dfd,stroke:#333,stroke-width:2px
```

## <Icon name="key" /> Key Components

### 1. [RouterService](/spooky/docs/developer/router_service) <Icon name="route" />

The orchestrator. It listens to events from all services and routes them to the appropriate destination based on a strict routing table. It is the only component that knows how services interact.

### 2. [SpookySync](/spooky/docs/developer/sync_service) <Icon name="sync" />

The bridge between Local and Remote. It is the **only** service (besides Auth) permitted to access the Remote Database. It handles:

- **Up Sync**: Pushing local mutations to the cloud.
- **Down Sync**: Applying remote changes to the local state.
- **Live Queries**: Listening for real-time updates from the server.

### 3. [QueryManager](/spooky/docs/developer/query_manager) <Icon name="book" />

The read layer. It manages "Incantations" (live queries) and allows the UI to subscribe to data. It reads **exclusively** from the Local Database.

### 4. [MutationManager](/spooky/docs/developer/mutation_manager) <Icon name="pen" />

The write layer. It handles all Create, Update, and Delete operations. It writes to the Local Database and logs mutations for synchronization.

### 5. [AuthService](/spooky/docs/developer/auth_service) <Icon name="lock" />

Manages authentication state and sessions with the Remote Database.

### 6. [StreamProcessorService](/spooky/docs/developer/stream_processor_service) <Icon name="wave" />

Stateful processor for converting `Incantation` definitions into continuously updating materialized views. It manages local persistence of query state and handles incremental updates.

### 7. [DevToolsService](/spooky/docs/developer/devtools_service) <Icon name="tool" />

A passive observer that exposes internal state to the Spooky Chrome Extension for debugging.
