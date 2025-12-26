import 'dart:io';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';
import '../schema/src/models.dart';

class SpookyController extends ChangeNotifier {
  SpookyClient? client;
  bool isLoggedIn = false;
  bool isInitializing = false;

  // UI Controllers
  final TextEditingController logController = TextEditingController();
  final ScrollController scrollController = ScrollController();

  final TextEditingController emailController = TextEditingController();
  final TextEditingController passwordController = TextEditingController();
  final TextEditingController namespaceController = TextEditingController(
    text: 'main',
  );
  final TextEditingController databaseController = TextEditingController(
    text: 'main',
  );
  final TextEditingController endpointController = TextEditingController(
    text: 'ws://127.0.0.1:8000/rpc',
  );

  // Dev Sidecar
  bool useDevSidecar = false;
  final TextEditingController devSidecarPortController = TextEditingController(
    text: '5000',
  );

  bool get isInitialized => client != null;

  @override
  void dispose() {
    logController.dispose();
    scrollController.dispose();
    emailController.dispose();
    passwordController.dispose();
    namespaceController.dispose();
    databaseController.dispose();
    endpointController.dispose();
    devSidecarPortController.dispose();
    client?.close();
    super.dispose();
  }

  void log(String message) {
    final now = DateTime.now()
        .toIso8601String()
        .split('T')
        .last
        .split('.')
        .first;
    logController.text = "[$now] $message\n${logController.text}";
    notifyListeners();
  }

  void toggleDevSidecar(bool? value) {
    useDevSidecar = value ?? false;
    notifyListeners();
  }

  Future<void> initSpooky() async {
    if (client != null) {
      log("Already initialized.");
      return;
    }
    if (isInitializing) {
      log("Initialization already in progress...");
      return;
    }

    try {
      isInitializing = true;
      notifyListeners();
      log("Initializing SpookyClient...");

      final dbPath = '${Directory.current.path}/db';
      await Directory(dbPath).create(recursive: true);

      final config = SpookyConfig(
        schemaSurql: SURQL_SCHEMA,
        schema: 'test_schema',
        database: DatabaseConfig(
          namespace: namespaceController.text,
          database: databaseController.text,
          path: dbPath,
          endpoint: endpointController.text.isEmpty
              ? null
              : endpointController.text,
          devSidecarPort: useDevSidecar
              ? int.tryParse(devSidecarPortController.text)
              : null,
        ),
      );

      final newClient = await SpookyClient.init(config);
      client = newClient;
      log("SpookyClient initialized successfully!");
    } catch (e, stack) {
      log("Error initializing: $e");
      debugPrintStack(stackTrace: stack);
    } finally {
      isInitializing = false;
      notifyListeners();
    }
  }

  Future<void> signIn({required Function(String) onError}) async {
    if (client == null) return;
    try {
      log("Attempting Sign In...");

      if (client!.remote.client == null) {
        log("Remote connection unavailable (Offline Mode). Cannot Sign In.");
        onError("Remote DB unavailable. Cannot Sign In.");
        return;
      }

      final credentials = jsonEncode({
        "username": emailController.text,
        "password": passwordController.text,
        "ns": namespaceController.text,
        "db": databaseController.text,
        "access": "account",
      });

      final token = await client!.remote.getClient.signin(
        credentialsJson: credentials,
      );
      log("Sign In Successful! Token: $token");
      isLoggedIn = true;
      notifyListeners();
    } catch (e) {
      log("Error Signing In: $e");
      onError("Sign In Failed: $e");
    }
  }

  Future<void> signUp({required Function(String) onError}) async {
    if (client == null) return;
    try {
      log("Attempting Sign Up (client.signup)...");

      if (client!.remote.getClient == null) {
        log("Remote connection unavailable (Offline Mode). Cannot Sign Up.");
        onError("Remote DB unavailable. Cannot Sign Up.");
        return;
      }

      final token = await client!.remote.manualSignup(
        username: emailController.text,
        password: passwordController.text,
        namespace: namespaceController.text,
        database: databaseController.text,
      );

      log("Sign Up Successful! Token: $token");
      isLoggedIn = true;
      notifyListeners();
    } catch (e) {
      log("Error Signing Up: $e");
      onError("Sign Up Failed: $e");
    }
  }

  Future<void> queryRemoteInfo() async {
    if (client == null) return;
    try {
      log("Querying Remote DB Info...");
      if (client!.remote.client == null) {
        log("Remote connection unavailable.");
        return;
      }
      final result = await client!.remote.getClient.query(
        sql: "INFO FOR DB;",
        vars: "{}",
      );
      log("Result: $result");
    } catch (e) {
      log("Error querying Remote: $e");
    }
  }

  Future<void> selectSchema() async {
    if (client == null) return;
    try {
      log("Selecting from user...");
      if (client!.remote.client == null) {
        log("Remote connection unavailable.");
        return;
      }
      final result = await client!.remote.getClient.query(
        sql: "SELECT * FROM user",
        vars: "{}",
      );
      log("Result: $result");
    } catch (e) {
      log("Error selecting schema: $e");
    }
  }

  Future<void> disconnect() async {
    log("Flushing & Closing DB...");
    try {
      final dumpPath = '${Directory.current.path}/db_dump.surql';
      await client?.local.export(dumpPath);
      log("DB flushed to $dumpPath");
      try {
        await File(dumpPath).delete();
      } catch (_) {}
    } catch (e) {
      log("Flush warning: $e");
    }

    await client?.close();
    client = null;
    isLoggedIn = false;
    log("Database Disconnected cleanly.");
    notifyListeners();
  }

  void logout() {
    isLoggedIn = false;
    log("Logged out.");
    notifyListeners();
  }
}
