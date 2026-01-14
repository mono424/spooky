# Flutter Core (Spooky)

The core logic library for the "Spooky" ecosystem, built on top of `flutter_surrealdb_engine`.

This package provides a high-level abstraction (`SpookyClient`) to manage local and remote SurrealDB instances, handle database migrations, and synchronize data.

## Features

- **Unified Client**: `SpookyClient` manages both local (embedded) and remote (cloud) database connections.
- **Automatic Migration**: `LocalMigration` applies your `schema.surql` to the local database upon initialization.
- **Dual-Database Architecture**: seamlessly switches or syncs between offline-first local storage and remote server.
- **Mutation Management**: Tracks and applies changes.

## Installation

Add `flutter_core` to your `pubspec.yaml`:

```yaml
dependencies:
  flutter_core:
    path: packages/flutter_core # or git url
```

## Usage

### 1. Configuration

Define your database configuration and schema.

```dart
import 'package:flutter_core/flutter_core.dart';

final config = SpookyConfig(
  schemaSurql: "DEFINE TABLE user SCHEMAFULL; DEFINE FIELD name ON user TYPE string;", 
  schema: "...", // Optional: JSON or other schema representation
  database: DatabaseConfig(
    path: '/path/to/local.db', // Path for local RocksDB
    namespace: 'test',
    database: 'test',
    endpoint: 'ws://localhost:8000', // Optional: Remote endpoint
    token: '...', // Optional: Authentication token
  ),
);
```

### 2. Initialization

Initialize the `SpookyClient`. This automatically:
1. Initializes the Rust engine.
2. Connects to the local database.
3. Connects to the remote database (if configured).
4. Applies the schema migration to the local database.

```dart
void main() async {
  final client = await SpookyClient.init(config);
  
  // App is ready
  runApp(MyApp(client: client));
}
```

### 3. Accessing Services

The `SpookyClient` exposes various services:

```dart
// Access local database directly
await client.local.db.select(resource: 'user');

// Access remote database directly
await client.remote.db.select(resource: 'user');

// Access mutation manager (for tracking changes)
client.mutation.create('user', {'name': 'Alice'});
```

### 4. Cleanup

Close connections when the app terminates.

```dart
await client.close();
```

## Architecture

The `SpookyClient` orchestrates several components:

- **LocalDatabaseService**: Wraps `flutter_surrealdb_engine` for on-device storage.
- **RemoteDatabaseService**: Wraps `flutter_surrealdb_engine` for remote server connection.
- **LocalMigration**: Handles schema provisioning.
- **MutationManager**: Manages data mutations.

## License

MIT
