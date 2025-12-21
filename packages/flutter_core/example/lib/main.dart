import 'dart:io';
import 'package:flutter/material.dart';
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

class _SpookyExampleAppState extends State<SpookyExampleApp> {
  SpookyClient? _client;
  final TextEditingController _logController = TextEditingController();
  final ScrollController _scrollController = ScrollController();

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

  Future<void> _initSpooky() async {
    if (_client != null) {
      _log("Already initialized.");
      return;
    }

    try {
      _log("Initializing SpookyClient...");

      final dbPath = '${Directory.current.path}/db';
      await Directory(dbPath).create(recursive: true);

      final config = SpookyConfig(
        schemaSurql: SURQL_SCHEMA,
        schema: 'test_schema', // Just a placeholder if unused by core logic yet
        database: DatabaseConfig(
          namespace: 'spooky_dev',
          database: 'spooky_db',
          path: dbPath,
          // Optional: Add endpoint if you have a remote server running
          // endpoint: 'ws://localhost:8000/rpc',
        ),
      );

      final client = await SpookyClient.init(config);

      setState(() {
        _client = client;
      });
      _log("SpookyClient initialized successfully!");
      _log("DB Path: $dbPath");
    } catch (e, stack) {
      _log("Error initializing: $e");
      debugPrintStack(stackTrace: stack);
    }
  }

  Future<void> _closeSpooky() async {
    if (_client == null) return;
    try {
      _log("Closing SpookyClient (triggers provisioning)...");
      await _client!.close();
      setState(() {
        _client = null;
      });
      _log("SpookyClient closed.");
    } catch (e) {
      _log("Error closing: $e");
    }
  }

  Future<void> _queryLocalInfo() async {
    if (_client == null) return;
    try {
      _log("Querying Local DB Info...");
      final result = await _client!.local.client!.queryDb(
        query: "INFO FOR DB;",
      );
      _log("Result: ${result.map((e) => e.result).join(', ')}");
    } catch (e) {
      _log("Error querying local: $e");
    }
  }

  Future<void> _selectSchema() async {
    if (_client == null) return;
    try {
      _log("Selecting from _spooky_schema...");
      final result = await _client!.local.client!.queryDb(
        query: "SELECT * FROM _spooky_schema;",
      );
      _log("Result: ${result.map((e) => e.result).join(', ')}");
    } catch (e) {
      _log("Error selecting schema: $e");
    }
  }

  Future<void> _runMigration() async {
    if (_client == null) return;
    try {
      _log("Manually running provision...");
      await _client!.migrator.provision(_client!.config.schemaSurql);
      _log("Provision complete.");
    } catch (e) {
      _log("Error running provision: $e");
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SpookyClient Testbed'),
        backgroundColor: Colors.deepPurple,
        foregroundColor: Colors.white,
      ),
      body: Column(
        children: [
          // Status Bar
          Container(
            padding: const EdgeInsets.all(12),
            color: _isInitialized ? Colors.green.shade100 : Colors.red.shade100,
            width: double.infinity,
            child: Text(
              _isInitialized ? "Status: CONNECTED" : "Status: DISCONNECTED",
              style: TextStyle(
                fontWeight: FontWeight.bold,
                color: _isInitialized
                    ? Colors.green.shade800
                    : Colors.red.shade800,
              ),
              textAlign: TextAlign.center,
            ),
          ),

          Expanded(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // Controls Area
                Expanded(
                  flex: 1,
                  child: ListView(
                    padding: const EdgeInsets.all(16),
                    children: [
                      const Text(
                        "Lifecycle",
                        style: TextStyle(fontWeight: FontWeight.bold),
                      ),
                      const Divider(),
                      ElevatedButton.icon(
                        onPressed: _isInitialized ? null : _initSpooky,
                        icon: const Icon(Icons.play_arrow),
                        label: const Text("Initialize"),
                        style: ElevatedButton.styleFrom(
                          backgroundColor: Colors.green,
                          foregroundColor: Colors.white,
                        ),
                      ),
                      const SizedBox(height: 8),
                      ElevatedButton.icon(
                        onPressed: _isInitialized ? _closeSpooky : null,
                        icon: const Icon(Icons.stop),
                        label: const Text("Close (Provision)"),
                        style: ElevatedButton.styleFrom(
                          backgroundColor: Colors.red,
                          foregroundColor: Colors.white,
                        ),
                      ),

                      const SizedBox(height: 24),
                      const Text(
                        "Local Operations",
                        style: TextStyle(fontWeight: FontWeight.bold),
                      ),
                      const Divider(),
                      ElevatedButton.icon(
                        onPressed: _isInitialized ? _queryLocalInfo : null,
                        icon: const Icon(Icons.info_outline),
                        label: const Text("INFO FOR DB"),
                      ),
                      const SizedBox(height: 8),
                      ElevatedButton.icon(
                        onPressed: _isInitialized ? _selectSchema : null,
                        icon: const Icon(Icons.schema_outlined),
                        label: const Text("Select _spooky_schema"),
                      ),
                      const SizedBox(height: 8),
                      ElevatedButton.icon(
                        onPressed: _isInitialized ? _runMigration : null,
                        icon: const Icon(Icons.build_circle_outlined),
                        label: const Text("Manual Provision"),
                        style: ElevatedButton.styleFrom(
                          backgroundColor: Colors.orange.shade100,
                          foregroundColor: Colors.black,
                        ),
                      ),
                    ],
                  ),
                ),

                const VerticalDivider(width: 1),

                // Logs Area
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
}
