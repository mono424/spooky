import 'dart:io';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart'; // Import for RecordId
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
  bool useDevSidecar = true;
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

      final schemaJson = jsonEncode({
        'relationships': [
          {'from': 'thread', 'field': 'author', 'to': 'user'},
        ],
      });

      final config = SpookyConfig(
        schemaSurql: SURQL_SCHEMA,
        schema: schemaJson,
        enableLiveQuery: true, // Enable full sync capabilities
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

      // PROVISION REMOTE (Dev Mode) - Removed root access as requested
      // if (useDevSidecar || endpointController.text.isNotEmpty) {
      //   log("Provisioning skipped (Root access removed).");
      // }

      log("SpookyClient initialized successfully!");
    } catch (e, stack) {
      log("Error initializing: $e");
      debugPrintStack(stackTrace: stack);
    } finally {
      isInitializing = false;
      notifyListeners();
    }
  }

  String? userId;

  Future<void> signIn(String username, String password) async {
    if (client == null) return;
    try {
      log("Attempting Sign In...");

      if (client!.remote.client == null) {
        log("Remote connection unavailable (Offline Mode). Cannot Sign In.");
        throw Exception("Remote DB unavailable. Cannot Sign In.");
      }

      final credentials = jsonEncode({
        "username": username,
        "password": password,
        "ns": namespaceController.text,
        "db": databaseController.text,
        "access": "account",
      });

      final token = await client!.remote.getClient.signin(creds: credentials);
      log("Sign In Successful! Token: $token");

      // Fetch User ID
      try {
        final authResult = await client!.remote.getClient.query(
          sql: "SELECT value id FROM \$auth",
        );

        final parsed = extractResult(authResult);
        if (parsed is List && parsed.isNotEmpty) {
          userId = parsed[0].toString();
          log("Authenticated as User ID: $userId");
        }
      } catch (e) {
        log("Warning: Could not fetch User ID: $e");
      }

      isLoggedIn = true;
      notifyListeners();
    } catch (e) {
      log("Error Signing In: $e");
      rethrow;
    }
  }

  Future<void> signUp(String username, String password) async {
    if (client == null) return;
    try {
      log("Attempting Sign Up (client.signup)...");

      if (client!.remote.getClient == null) {
        log("Remote connection unavailable (Offline Mode). Cannot Sign Up.");
        throw Exception("Remote DB unavailable. Cannot Sign Up.");
      }

      final token = await client!.remote.manualSignup(
        username: username,
        password: password,
        namespace: namespaceController.text,
        database: databaseController.text,
      );

      log("Sign Up Successful! Token: $token");
      isLoggedIn = true;
      notifyListeners();
    } catch (e) {
      log("Error Signing Up: $e");
      rethrow;
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

  // --- Proxy Methods for SpookyClient ---

  Future<MutationResponse> create(
    RecordId id,
    Map<String, dynamic> data,
  ) async {
    if (client == null) throw Exception("Client not initialized");
    return client!.create(id, data);
  }

  Future<MutationResponse> update(
    RecordId id,
    Map<String, dynamic> data,
  ) async {
    if (client == null) throw Exception("Client not initialized");
    return client!.update(id, data);
  }

  Future<void> delete(RecordId id) async {
    if (client == null) throw Exception("Client not initialized");
    return client!.delete(id);
  }
}
