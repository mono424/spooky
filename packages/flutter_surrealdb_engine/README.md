# Flutter SurrealDB Engine

A powerful, high-performance Flutter plugin that embeds **SurrealDB** directly into your application using Rust and Dart FFI.

`flutter_surrealdb_engine` allows you to run a full SurrealDB instance locally on your device (iOS, Android, macOS, Windows, Linux) without needing a separate server, or connect to a remote instance with the same API.

## Features

- 🚀 **Embedded SurrealDB**: Run SurrealDB in-memory or on-disk directly within your Flutter app.
- 🌐 **Remote Connection**: Connect to remote SurrealDB instances via WebSocket/HTTP.
- ⚡ **High Performance**: Built with Rust and `flutter_rust_bridge` for near-native performance.
- 🛠 **Dev Sidecar**: Spawn a local sidecar server for development and debugging.
- 🔒 **Authentication**: Built-in support for Signup, Signin, and JWT authentication.
- 📦 **ACID Transactions**: Full support for transactions and complex queries.

## Architecture

```mermaid
graph TD
    Dart[Flutter App (Dart)]
    FFI[Dart FFI Bridge]
    Rust[Rust Engine (flutter_surrealdb_engine)]
    Surreal[SurrealDB Instance]

    Dart <-->|Calls| FFI
    FFI <-->|Invokes| Rust
    Rust <-->|Embeds/Connects| Surreal
```

## Installation

Add `flutter_surrealdb_engine` to your `pubspec.yaml`:

```yaml
dependencies:
  flutter_surrealdb_engine:
    path: packages/flutter_surrealdb_engine # or git url
```

## Usage

### 1. Initialization

Before using the library, you must initialize the Rust bridge. This is typically done in your `main()` function.

```dart
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

void main() async {
  // Initialize the Rust bridge
  await RustLib.init();
  
  runApp(const MyApp());
}
```

### 2. Connecting to a Database

You can connect to an in-memory database, a local file-based database, or a remote server.

#### In-Memory (Ephemeral)
Great for testing or temporary data.

```dart
final db = await SurrealDb.connect(
  mode: const StorageMode.memory(),
);
```

#### File-Based (Persistent)
Stores data locally on the device.

```dart
final db = await SurrealDb.connect(
  mode: const StorageMode.disk(path: '/path/to/database.db'),
);
```

#### Remote Server
Connect to an existing SurrealDB instance.

```dart
final db = await SurrealDb.connect(
  mode: const StorageMode.remote(url: 'ws://localhost:8000'),
);
```

### 3. Authentication & Setup

Once connected, you typically need to sign in and select a namespace/database.

```dart
// Select Namespace and Database
await db.useDb(namespace: 'test', database: 'test');

// Sign Up (Create new user)
final token = await db.signup(
  credentialsJson: '{"user": "root", "pass": "root", "NS": "test", "DB": "test", "SC": "user"}',
);

// Sign In (Existing user)
final token = await db.signin(
  credentialsJson: '{"user": "root", "pass": "root", "NS": "test", "DB": "test", "SC": "user"}',
);
```

### 4. CRUD Operations

Perform standard Create, Read, Update, Delete operations.

```dart
// Create a new record
final created = await db.create(
  resource: 'person',
  data: '{"name": "John Doe", "age": 30}',
);

// Select records
final people = await db.select(resource: 'person');

// Update a record
final updated = await db.update(
  resource: 'person:ow485n29',
  data: '{"name": "John Smith", "age": 31}',
);

// Delete a record
await db.delete(resource: 'person:ow485n29');
```

### 5. Custom Queries

Execute raw SurrealQL queries.

```dart
final result = await db.query(
  sql: 'SELECT * FROM person WHERE age > \$age',
  vars: '{"age": 25}',
);
```

### 6. Transactions

You can run multiple queries within a single transaction.

```dart
await db.transaction(
  statements: '''
    CREATE person:1 CONTENT { name: 'Alice' };
    CREATE person:2 CONTENT { name: 'Bob' };
    RELATE person:1->knows->person:2;
  ''',
);
```

## API Reference

| Method | Description |
|--------|-------------|
| `connect` | Establishes a connection to the database (Memory, Disk, or Remote). |
| `useDb` | Selects the namespace and database to use. |
| `signup` | Registers a new user and returns a sequence token. |
| `signin` | Authenticates an existing user and returns a token. |
| `invalidate` | Invalidates the current session/authentication. |
| `query` | Executes a raw SurrealQL query with optional variables. |
| `select` | Selects all records from a table or a specific record ID. |
| `create` | Creates a new record. |
| `update` | Updates an existing record. |
| `merge` | Merges data into an existing record. |
| `delete` | Deletes a record or table. |
| `export_` | Exports the database to a file (only for local instances). |

## License

MIT
