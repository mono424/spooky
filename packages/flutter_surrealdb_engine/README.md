# Flutter SurrealDB Engine

A Flutter plugin that embeds SurrealDB using the Rust engine (`kv-rocksdb`). This plugin allows you to run a local SurrealDB instance directly within your Flutter application, communicating via Dart FFI.

## Features

- **Embedded SurrealDB**: Runs a local `rocksdb` backed SurrealDB instance.
- **Direct SurrealQL**: Execute any SurrealQL query using `queryDb`.
- **Authentication**: Support for Root, Namespace, Database, and Token authentication.
- **Session Management**: Switch Namespaces and Databases dynamically.
- **Performance Metrics**: All operations return execution time duration.
- **Standardized API**: Consistent `SurrealResult` return type for all operations.

## Prerequisites

- **Flutter SDK**: [Install Flutter](https://flutter.dev/docs/get-started/install)
- **Rust Toolchain**: [Install Rust](https://www.rust-lang.org/tools/install)
- **Flutter Rust Bridge Codegen**:
  ```bash
  cargo install flutter_rust_bridge_codegen
  ```

## Setup & Installation

1.  **Clone the repository**:
    ```bash
    git clone <repository_url>
    cd flutter_surrealdb_engine
    ```

2.  **Install Dart dependencies**:
    ```bash
    flutter pub get
    ```

3.  **Generate Rust Bindings**:
    This step is required if you modify the Rust code or if the bindings are missing.
    ```bash
    flutter_rust_bridge_codegen generate
    ```

## Running the Example App

The example application provides a complete UI to test all plugin features, including connection, authentication, and CRUD operations.

1.  Navigate to the example directory:
    ```bash
    cd example
    ```

2.  Run the app (e.g., on macOS):
    ```bash
    flutter run -d macos
    ```

## Running Tests

Integration tests are located in `example/integration_test/plugin_integration_test.dart`. These tests verify the plugin's functionality on a real device or simulator.

To run the tests:

```bash
cd example
flutter test integration_test/plugin_integration_test.dart -d macos
```

## Project Structure

- **`lib/`**: Contains the Dart plugin code and generated FFI bindings.
- **`rust/`**: Contains the Rust source code.
    - `src/lib.rs`: The main Rust API implementation.
- **`example/`**: A Flutter application demonstrating usage.
    - `lib/main.dart`: The main UI for the demo app.
    - `integration_test/`: Contains integration tests.

## API Usage

### Connecting
```dart
SurrealDatabase db = await connectDb(path: 'path/to/database.db');
```

### Executing Queries
```dart
final results = await db.queryDb(query: "SELECT * FROM person");
for (var result in results) {
  print(result.status);
  print(result.time);
  print(result.result); // JSON string
}
```

### Authentication
```dart
// Signin
final res = await db.signinRoot(username: 'root', password: 'root');
final token = res.first.result; // JSON string of token

// Authenticate with token
await db.authenticate(token: token);
```
