import 'dart:io';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_core/core/services/database/surreal_decoder.dart';
import 'package:flutter_core/flutter_core.dart';
import 'schema/src/models.dart';

void main() {
  runApp(const MaterialApp(home: SpookyExampleApp()));
}

class SpookyExampleApp extends StatefulWidget {
  const SpookyExampleApp({super.key});

  @override
  State<SpookyExampleApp> createState() => _SpookyExampleAppState();
}

class _SpookyExampleAppState extends State<SpookyExampleApp>
    with WidgetsBindingObserver {
  SpookyClient? _client;
  bool _isLoggedIn = false;

  final TextEditingController _logController = TextEditingController();
  final ScrollController _scrollController = ScrollController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    // Try to close nicely if widget is disposed
    _client?.close();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.detached) {
      // Ensure we flush DB on exit!
      _client?.close();
    }
  }

  final TextEditingController _emailController = TextEditingController();
  final TextEditingController _passwordController = TextEditingController();
  final TextEditingController _namespaceController = TextEditingController(
    text: 'main',
  );
  final TextEditingController _databaseController = TextEditingController(
    text: 'main',
  );
  final TextEditingController _endpointController = TextEditingController(
    text: 'ws://127.0.0.1:8000/rpc',
  );

  bool _useDevSidecar = false;
  final TextEditingController _devSidecarPortController = TextEditingController(
    text: '5000',
  );

  bool get _isInitialized => _client != null;

  void _log(String message) {
    if (!mounted) return;
    setState(() {
      final now = DateTime.now()
          .toIso8601String()
          .split('T')
          .last
          .split('.')
          .first;
      _logController.text = "[$now] $message\n${_logController.text}";
    });
  }

  bool _isInitializing = false;

  Future<void> _initSpooky() async {
    if (_client != null) {
      _log("Already initialized.");
      return;
    }
    if (_isInitializing) {
      _log("Initialization already in progress...");
      return;
    }

    try {
      _isInitializing = true;
      _log("Initializing SpookyClient...");

      final dbPath = '${Directory.current.path}/db';
      await Directory(dbPath).create(recursive: true);

      final config = SpookyConfig(
        schemaSurql: SURQL_SCHEMA,
        schema: 'test_schema',
        database: DatabaseConfig(
          namespace: _namespaceController.text,
          database: _databaseController.text,
          path: dbPath,
          endpoint: _endpointController.text.isEmpty
              ? null
              : _endpointController.text,
          devSidecarPort: _useDevSidecar
              ? int.tryParse(_devSidecarPortController.text)
              : null,
        ),
      );

      final client = await SpookyClient.init(config);
      await client.createEvent();

      setState(() {
        _client = client;
      });
      _log("SpookyClient initialized successfully!");
    } catch (e, stack) {
      _log("Error initializing: $e");
      debugPrintStack(stackTrace: stack);
    } finally {
      _isInitializing = false;
    }
  }

  Future<void> _signIn() async {
    if (_client == null) return;
    try {
      _log("Attempting Sign In...");

      if (_client!.remote.client == null) {
        _log("Remote connection unavailable (Offline Mode). Cannot Sign In.");
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text("Remote DB unavailable. Cannot Sign In."),
              backgroundColor: Colors.orange,
            ),
          );
        }
        return;
      }

      // Using Remote client for demo purposes as default config has no remote endpoint.
      // Using Remote client for demo purposes as default config has no remote endpoint.
      final credentials = jsonEncode({
        "username": _emailController.text,
        "password": _passwordController.text,
        "ns": _namespaceController.text,
        "db": _databaseController.text,
        "access": "account", // v3 uses 'access', matches 'Record' variant
      });

      final token = await _client!.remote.getClient.signin(
        credentialsJson: credentials,
      );

      _log("Sign In Successful! Token: $token");

      setState(() {
        _isLoggedIn = true;
      });
    } catch (e) {
      _log("Error Signing In: $e");
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Sign In Failed: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  Future<void> _signUp() async {
    if (_client == null) return;
    try {
      _log("Attempting Sign Up (client.signup)...");

      if (_client!.remote.getClient == null) {
        _log("Remote connection unavailable (Offline Mode). Cannot Sign Up.");
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text("Remote DB unavailable. Cannot Sign Up."),
              backgroundColor: Colors.orange,
            ),
          );
        }
        return;
      }

      final token = await _client!.remote.manualSignup(
        username: _emailController.text,
        password: _passwordController.text,
        namespace: _namespaceController.text,
        database: _databaseController.text,
      );

      _log("Sign Up Successful! Token: $token");

      // Signup usually returns a token, effectively signing the user in.
      setState(() {
        _isLoggedIn = true;
      });
    } catch (e) {
      _log("Error Signing Up: $e");
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Sign Up Failed: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  Future<void> _queryRemoteInfo() async {
    if (_client == null) return;
    try {
      _log("Querying Remote DB Info...");
      if (_client!.remote.client == null) {
        _log("Remote connection unavailable.");
        return;
      }
      final result = await _client!.remote.getClient.query(
        sql: "INFO FOR DB;",
        vars: "{}",
      );
      _log("Result: $result");
    } catch (e) {
      _log("Error querying Remote: $e");
    }
  }

  Future<void> _selectSchema() async {
    if (_client == null) return;
    try {
      _log("Selecting from _spooky_schema...");
      if (_client!.remote.client == null) {
        _log("Remote connection unavailable.");
        return;
      }
      final result = await _client!.remote.getClient.query(
        sql: "SELECT * FROM user",
        vars: "{}",
      );
      _log("Result: $result");
    } catch (e) {
      _log("Error selecting schema: $e");
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SpookyClient Testbed'),
        backgroundColor: Colors.deepPurple,
        foregroundColor: Colors.white,
        actions: [
          IconButton(
            icon: const Icon(Icons.power_settings_new),
            tooltip: "Disconnect / Close DB",
            onPressed: () async {
              _log("Flushing & Closing DB...");

              // Force export/flush before closing (workaround for embedded persistence)
              // Dump to a temp file we don't care about, just to trigger the engine snapshot
              try {
                final dumpPath = '${Directory.current.path}/db_dump.surql';
                await _client?.local.export(dumpPath);
                _log("DB flushed to $dumpPath");
                try {
                  await File(dumpPath).delete();
                } catch (_) {}
              } catch (e) {
                _log("Flush warning: $e");
              }

              await _client?.close();
              setState(() {
                _client = null;
                _isLoggedIn = false;
              });
              _log("Database Disconnected cleanly.");
            },
          ),
          if (_isLoggedIn)
            IconButton(
              icon: const Icon(Icons.logout),
              onPressed: () {
                setState(() {
                  _isLoggedIn = false;
                });
                _log("Logged out.");
              },
            ),
        ],
      ),
      body: Column(
        children: [
          // Connection Status Bar
          Container(
            padding: const EdgeInsets.all(8),
            color: _isInitialized ? Colors.green.shade100 : Colors.red.shade100,
            width: double.infinity,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Icon(
                  _client != null ? Icons.check_circle : Icons.error,
                  size: 16,
                  color: _isInitialized
                      ? Colors.green.shade800
                      : Colors.red.shade800,
                ),
                const SizedBox(width: 8),
                Text(
                  _isInitialized
                      ? "Client Initialized"
                      : "Client Not Initialized",
                  style: TextStyle(
                    fontWeight: FontWeight.bold,
                    color: _isInitialized
                        ? Colors.green.shade800
                        : Colors.red.shade800,
                  ),
                ),
              ],
            ),
          ),

          Expanded(
            child: Row(
              children: [
                // Main Content Area
                Expanded(
                  flex: 3,
                  child: Padding(
                    padding: const EdgeInsets.all(24.0),
                    child: _buildMainContent(),
                  ),
                ),

                const VerticalDivider(width: 1),

                // Logs Sidebar
                Expanded(
                  flex: 2,
                  child: Column(
                    children: [
                      Container(
                        padding: const EdgeInsets.all(8),
                        color: Colors.grey.shade200,
                        width: double.infinity,
                        child: const Text(
                          "Logs",
                          style: TextStyle(fontWeight: FontWeight.bold),
                        ),
                      ),
                      Expanded(
                        child: Container(
                          color: Colors.black87,
                          child: TextField(
                            controller: _logController,
                            scrollController: _scrollController,
                            readOnly: true,
                            maxLines: null,
                            style: const TextStyle(
                              color: Colors.greenAccent,
                              fontFamily: 'Courier',
                              fontSize: 13,
                            ),
                            decoration: const InputDecoration(
                              contentPadding: EdgeInsets.all(12),
                              border: InputBorder.none,
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildMainContent() {
    if (!_isInitialized) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Text(
              "Initialize Spooky Client to Begin",
              style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 20),
            SizedBox(
              width: 300,
              child: TextField(
                controller: _namespaceController,
                decoration: const InputDecoration(
                  labelText: "Namespace",
                  border: OutlineInputBorder(),
                ),
              ),
            ),
            const SizedBox(height: 10),
            SizedBox(
              width: 300,
              child: TextField(
                controller: _databaseController,
                decoration: const InputDecoration(
                  labelText: "Database",
                  border: OutlineInputBorder(),
                ),
              ),
            ),
            const SizedBox(height: 10),
            SizedBox(
              width: 300,
              child: TextField(
                controller: _endpointController,
                decoration: const InputDecoration(
                  labelText: "Endpoint (Optional)",
                  hintText: "ws://127.0.0.1:8000/rpc",
                  border: OutlineInputBorder(),
                ),
              ),
            ),
            const SizedBox(height: 10),
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Checkbox(
                  value: _useDevSidecar,
                  onChanged: (val) {
                    setState(() {
                      _useDevSidecar = val ?? false;
                    });
                  },
                ),
                const Text("Enable Dev Sidecar (Host Local Server)"),
              ],
            ),
            if (_useDevSidecar) ...[
              SizedBox(
                width: 300,
                child: TextField(
                  controller: _devSidecarPortController,
                  decoration: const InputDecoration(
                    labelText: "Sidecar Port",
                    hintText: "5000",
                    border: OutlineInputBorder(),
                  ),
                  keyboardType: TextInputType.number,
                ),
              ),
              const SizedBox(height: 8),
              const Text(
                "Credentials: root / root",
                style: TextStyle(
                  fontStyle: FontStyle.italic,
                  color: Colors.grey,
                ),
              ),
            ],
            const SizedBox(height: 20),
            ElevatedButton.icon(
              onPressed: _initSpooky,
              icon: const Icon(Icons.play_arrow),
              label: const Text("Initialize Client"),
              style: ElevatedButton.styleFrom(
                padding: const EdgeInsets.symmetric(
                  horizontal: 32,
                  vertical: 16,
                ),
              ),
            ),
          ],
        ),
      );
    }

    if (!_isLoggedIn) {
      return Center(
        child: Container(
          constraints: const BoxConstraints(maxWidth: 400),
          child: Card(
            elevation: 4,
            child: Padding(
              padding: const EdgeInsets.all(24.0),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Text(
                    "Welcome Back",
                    style: TextStyle(fontSize: 24, fontWeight: FontWeight.bold),
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 24),
                  TextField(
                    controller: _emailController,
                    decoration: const InputDecoration(
                      labelText: "Email / User",
                      border: OutlineInputBorder(),
                      prefixIcon: Icon(Icons.person),
                    ),
                  ),
                  const SizedBox(height: 16),
                  TextField(
                    controller: _passwordController,
                    decoration: const InputDecoration(
                      labelText: "Password",
                      border: OutlineInputBorder(),
                      prefixIcon: Icon(Icons.lock),
                    ),
                    obscureText: true,
                  ),
                  const SizedBox(height: 24),
                  ElevatedButton(
                    onPressed: _signIn,
                    style: ElevatedButton.styleFrom(
                      padding: const EdgeInsets.symmetric(vertical: 16),
                      backgroundColor: Colors.deepPurple,
                      foregroundColor: Colors.white,
                    ),
                    child: const Text("Sign In"),
                  ),
                  const SizedBox(height: 12),
                  OutlinedButton(
                    onPressed: _signUp,
                    style: OutlinedButton.styleFrom(
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                    child: const Text("Sign Up"),
                  ),
                  const SizedBox(height: 16),
                ],
              ),
            ),
          ),
        ),
      );
    }

    return ListView(
      children: [
        const Text(
          "Dashboard",
          style: TextStyle(fontWeight: FontWeight.bold, fontSize: 24),
        ),
        const SizedBox(height: 20),
        Wrap(
          spacing: 16,
          runSpacing: 16,
          children: [
            _buildActionCard(
              title: "Remote DB Info",
              icon: Icons.info_outline,
              onTap: _queryRemoteInfo,
              color: Colors.blue.shade50,
            ),
            _buildActionCard(
              title: "Schema Query",
              icon: Icons.schema_outlined,
              onTap: _selectSchema,
              color: Colors.orange.shade50,
            ),
          ],
        ),
      ],
    );
  }

  Widget _buildActionCard({
    required String title,
    required IconData icon,
    required VoidCallback onTap,
    Color? color,
    Color? textColor,
  }) {
    return InkWell(
      onTap: onTap,
      child: Container(
        width: 160,
        height: 120,
        decoration: BoxDecoration(
          color: color ?? Colors.white,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: Colors.grey.shade300),
        ),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(icon, size: 32, color: textColor ?? Colors.deepPurple),
            const SizedBox(height: 8),
            Text(
              title,
              style: TextStyle(
                fontWeight: FontWeight.w600,
                color: textColor ?? Colors.black87,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
