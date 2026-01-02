import 'package:flutter/material.dart';
import 'dart:convert';
import 'package:path_provider/path_provider.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  try {
    await RustLib.init();
    runApp(const MyApp());
  } catch (e, stack) {
    print("CRITICAL STARTUP ERROR: $e");
    print(stack);
    runApp(ErrorApp(error: e.toString(), stack: stack.toString()));
  }
}

class ErrorApp extends StatelessWidget {
  final String error;
  final String stack;
  const ErrorApp({super.key, required this.error, required this.stack});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          padding: const EdgeInsets.all(16),
          child: Column(
            children: [
              const Icon(Icons.error, color: Colors.red, size: 64),
              const Text("Startup Error", style: TextStyle(fontSize: 24)),
              Text(error, style: const TextStyle(color: Colors.red)),
              const Divider(),
              Text(stack),
            ],
          ),
        ),
      ),
    );
  }
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'SurrealDB Example',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.purple),
        useMaterial3: true,
        inputDecorationTheme: const InputDecorationTheme(
          border: OutlineInputBorder(),
          isDense: true,
        ),
      ),
      home: const HomePage(),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  SurrealDb? _db;
  bool _isLoading = false;
  final StringBuilder _logs = StringBuilder();
  final ScrollController _logScrollController = ScrollController();
  String? _authToken;

  // Controllers
  final _nsCtrl = TextEditingController(text: "test");
  final _dbCtrl = TextEditingController(text: "test");

  final _userCtrl = TextEditingController(text: "root");
  final _passCtrl = TextEditingController(text: "root");
  final _scopeCtrl = TextEditingController(text: "");

  final _resourceCtrl = TextEditingController(text: "person");
  final _dataCtrl = TextEditingController(text: '{"name": "Tobie", "age": 30}');

  final _queryCtrl = TextEditingController(text: "RETURN time::now();");
  final _varsCtrl = TextEditingController(text: "{}");

  void _log(String message) {
    setState(() {
      _logs.writeln(
        "[${DateTime.now().toIso8601String().substring(11, 19)}] $message",
      );
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_logScrollController.hasClients) {
        _logScrollController.jumpTo(
          _logScrollController.position.maxScrollExtent,
        );
      }
    });
  }

  Future<void> _safeCall(Future<dynamic> Function() action) async {
    if (_db == null) {
      _log("Error: Database not connected");
      return;
    }
    setState(() => _isLoading = true);
    try {
      final res = await action();
      if (res != null) {
        _log("Result: $res");
      } else {
        _log("Success");
      }
    } catch (e) {
      _log("Error: $e");
    } finally {
      setState(() => _isLoading = false);
    }
  }

  // --- Connection ---
  Future<void> _connect(bool disk) async {
    if (_db != null) return;
    setState(() => _isLoading = true);
    try {
      if (disk) {
        final dir = await getApplicationDocumentsDirectory();
        final path = "${dir.path}/app.db";
        _log("Connecting to Disk: $path");
        _db = await SurrealDb.connect(mode: StorageMode.disk(path: path));
      } else {
        _log("Connecting to Memory");
        _db = await SurrealDb.connect(mode: const StorageMode.memory());
      }
      _log("Connected!");
      await _useDb();
    } catch (e) {
      _log("Connection Error: $e");
    } finally {
      setState(() => _isLoading = false);
    }
  }

  Future<void> _disconnect() async {
    if (_db == null) return;
    try {
      await _db!.close();
      _db = null;
      _authToken = null;
      _log("Disconnected");
    } catch (e) {
      _log("Error closing: $e");
    } finally {
      setState(() {});
    }
  }

  Future<void> _useDb() async {
    await _safeCall(() async {
      await _db!.useDb(namespace: _nsCtrl.text, database: _dbCtrl.text);
      return "Switched to NS: ${_nsCtrl.text}, DB: ${_dbCtrl.text}";
    });
  }

  // --- Auth ---
  Future<void> _signin() async {
    await _safeCall(() async {
      final creds = {"user": _userCtrl.text, "pass": _passCtrl.text};
      if (_scopeCtrl.text.isNotEmpty) {
        creds["sc"] = _scopeCtrl.text; // Scope
        creds.remove("user");
        creds.remove("pass");
        // Add extra params if needed for scope signin
        final extra = jsonDecode(
          _dataCtrl.text,
        ); // Use data field for extra params
        creds.addAll(extra);
      }
      // Basic root/ns/db auth
      if (_scopeCtrl.text.isEmpty) {
        // usually we default to root auth if no scope.
        // The implementation in auth.rs expects specific fields.
        // Let's pass basic user/pass map.
      }

      final token = await _db!.signin(credentialsJson: jsonEncode(creds));
      setState(() => _authToken = token);
      return "Signed In. Token: ${token.substring(0, 10)}...";
    });
  }

  Future<void> _signup() async {
    await _safeCall(() async {
      final creds = {"user": _userCtrl.text, "pass": _passCtrl.text};
      // Signup typically requires scope/ns/db levels
      final token = await _db!.signup(credentialsJson: jsonEncode(creds));
      setState(() => _authToken = token);
      return "Signed Up. Token: ${token.substring(0, 10)}...";
    });
  }

  Future<void> _authenticate() async {
    if (_authToken == null) {
      _log("No token available to authenticate with.");
      return;
    }
    await _safeCall(() async {
      await _db!.authenticate(token: _authToken!);
      return "Authenticated with existing token.";
    });
  }

  Future<void> _invalidate() async {
    await _safeCall(() async {
      await _db!.invalidate();
      setState(() => _authToken = null);
      return "Session invalidated.";
    });
  }

  // --- CRUD ---
  Future<void> _select() async {
    await _safeCall(() => _db!.select(resource: _resourceCtrl.text));
  }

  Future<void> _create() async {
    await _safeCall(
      () => _db!.create(resource: _resourceCtrl.text, data: _dataCtrl.text),
    );
  }

  Future<void> _update() async {
    await _safeCall(
      () => _db!.update(resource: _resourceCtrl.text, data: _dataCtrl.text),
    );
  }

  Future<void> _merge() async {
    await _safeCall(
      () => _db!.merge(resource: _resourceCtrl.text, data: _dataCtrl.text),
    );
  }

  Future<void> _delete() async {
    await _safeCall(() => _db!.delete(resource: _resourceCtrl.text));
  }

  // --- Query ---
  Future<void> _runQuery() async {
    await _safeCall(
      () => _db!.query(sql: _queryCtrl.text, vars: _varsCtrl.text),
    );
  }

  Future<void> _testAll() async {
    if (_db != null) await _disconnect();

    _log("=== STARTING COMPREHENSIVE TEST ===");

    // 1. Connect
    await _connect(false);
    if (_db == null) return;

    // 2. Auth (Root)
    await _safeCall(() async {
      // Signin as root
      final creds = {"user": "root", "pass": "root"};
      final token = await _db!.signin(credentialsJson: jsonEncode(creds));
      return "Root Signin OK. Token len: ${token.length}";
    });

    // 3. Switch DB
    await _useDb();

    // 4. Create
    await _safeCall(() async {
      return await _db!.create(
        resource: "person:tobie",
        data: jsonEncode({"name": "Tobie", "age": 30, "marketing": true}),
      );
    });

    // 5. Select
    await _safeCall(() async {
      final res = await _db!.select(resource: "person:tobie");
      if (!res.contains("Tobie")) throw "Select failed to find data";
      return "Select OK: $res";
    });

    // 6. Update
    await _safeCall(() async {
      return await _db!.update(
        resource: "person:tobie",
        data: jsonEncode({"name": "Tobie", "age": 31}),
      );
    });

    // 7. Query
    await _safeCall(() async {
      final res = await _db!.query(
        sql: "SELECT * FROM person WHERE age > \$min_age",
        vars: jsonEncode({"min_age": 20}),
      );
      return "Query OK: $res";
    });

    // 8. Transaction
    await _safeCall(() async {
      final stmts = jsonEncode([
        "CREATE account:1 SET balance = 100",
        "CREATE account:2 SET balance = 100",
        "UPDATE account:1 SET balance = 50",
      ]);
      final res = await _db!.transaction(statements: stmts, vars: "{}");
      return "Transaction OK: $res";
    });

    _log("=== TEST COMPLETE ===");
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("SurrealDB Flutter")),
      body: Column(
        children: [
          Expanded(
            flex: 3,
            child: ListView(
              padding: const EdgeInsets.all(12),
              children: [
                // 1. Connection
                _buildSection("Connection", [
                  Row(
                    children: [
                      Expanded(
                        child: ElevatedButton(
                          onPressed: _db == null ? () => _connect(false) : null,
                          child: const Text("Mem"),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: ElevatedButton(
                          onPressed: _db == null ? () => _connect(true) : null,
                          child: const Text("Disk"),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: ElevatedButton(
                          onPressed: _db != null ? _disconnect : null,
                          style: ElevatedButton.styleFrom(
                            backgroundColor: Colors.red.shade100,
                          ),
                          child: const Text("Close"),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Row(
                    children: [
                      Expanded(
                        child: TextField(
                          controller: _nsCtrl,
                          decoration: const InputDecoration(
                            labelText: "Namespace",
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: TextField(
                          controller: _dbCtrl,
                          decoration: const InputDecoration(
                            labelText: "Database",
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      IconButton(
                        onPressed: _db != null ? _useDb : null,
                        icon: const Icon(Icons.check),
                      ),
                    ],
                  ),
                ]),

                // 2. Auth
                _buildSection("Authentication", [
                  Row(
                    children: [
                      Expanded(
                        child: TextField(
                          controller: _userCtrl,
                          decoration: const InputDecoration(labelText: "User"),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: TextField(
                          controller: _passCtrl,
                          decoration: const InputDecoration(labelText: "Pass"),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: TextField(
                          controller: _scopeCtrl,
                          decoration: const InputDecoration(
                            labelText: "Scope (opt)",
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 8,
                    children: [
                      ElevatedButton(
                        onPressed: _db != null ? _signin : null,
                        child: const Text("Signin"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _signup : null,
                        child: const Text("Signup"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _authenticate : null,
                        child: const Text("Auth Token"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _invalidate : null,
                        child: const Text("Invalidate"),
                      ),
                    ],
                  ),
                  if (_authToken != null)
                    Text(
                      "Token: ${_authToken!.substring(0, 10)}...",
                      style: const TextStyle(fontSize: 10, color: Colors.green),
                    ),
                ]),

                // 3. CRUD
                _buildSection("CRUD", [
                  Row(
                    children: [
                      Expanded(
                        flex: 1,
                        child: TextField(
                          controller: _resourceCtrl,
                          decoration: const InputDecoration(
                            labelText: "Resource (thing)",
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        flex: 2,
                        child: TextField(
                          controller: _dataCtrl,
                          decoration: const InputDecoration(
                            labelText: "Data (JSON)",
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 8,
                    children: [
                      ElevatedButton(
                        onPressed: _db != null ? _select : null,
                        child: const Text("Select"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _create : null,
                        child: const Text("Create"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _update : null,
                        child: const Text("Update"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _merge : null,
                        child: const Text("Merge"),
                      ),
                      ElevatedButton(
                        onPressed: _db != null ? _delete : null,
                        child: const Text("Delete"),
                      ),
                    ],
                  ),
                ]),

                // 4. Query
                _buildSection("Raw Query", [
                  TextField(
                    controller: _queryCtrl,
                    maxLines: 3,
                    decoration: const InputDecoration(labelText: "SQL"),
                  ),
                  const SizedBox(height: 8),
                  TextField(
                    controller: _varsCtrl,
                    decoration: const InputDecoration(labelText: "Vars (JSON)"),
                  ),
                  const SizedBox(height: 8),
                  ElevatedButton(
                    onPressed: _db != null ? _runQuery : null,
                    child: const Text("Run Query"),
                  ),
                ]),

                // 5. Test All
                _buildSection("Comprehensive Test", [
                  ElevatedButton.icon(
                    onPressed: _testAll,
                    style: ElevatedButton.styleFrom(
                      backgroundColor: Colors.green.shade100,
                      padding: const EdgeInsets.all(16),
                    ),
                    icon: const Icon(Icons.playlist_play),
                    label: const Text("Run All Tests (Mem)"),
                  ),
                ]),
              ],
            ),
          ),

          if (_isLoading) const LinearProgressIndicator(),

          // Logs
          Expanded(
            flex: 2,
            child: Container(
              color: Colors.black87,
              width: double.infinity,
              padding: const EdgeInsets.all(8),
              child: SingleChildScrollView(
                controller: _logScrollController,
                child: Text(
                  _logs.toString(),
                  style: const TextStyle(
                    fontFamily: 'Courier',
                    fontSize: 12,
                    color: Colors.white70,
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildSection(String title, List<Widget> children) {
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              title,
              style: const TextStyle(fontWeight: FontWeight.bold, fontSize: 16),
            ),
            const Divider(),
            ...children,
          ],
        ),
      ),
    );
  }
}

class StringBuilder {
  final StringBuffer _buffer = StringBuffer();
  void writeln(String s) => _buffer.writeln(s);
  @override
  String toString() => _buffer.toString();
}
