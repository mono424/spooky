import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import '../../controllers/spooky_controller.dart';
import '../../core/theme.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

// Modular Widgets
import 'widgets/live_query_controls.dart';
import 'widgets/live_query_logs.dart';
import 'live_query_view.dart';

class LiveQueryDashboard extends StatefulWidget {
  final SpookyController controller;

  const LiveQueryDashboard({super.key, required this.controller});

  @override
  State<LiveQueryDashboard> createState() => _LiveQueryDashboardState();
}

class _LiveQueryDashboardState extends State<LiveQueryDashboard> {
  final TextEditingController _tableController = TextEditingController(
    text: 'user',
  );
  StreamSubscription<LiveQueryEvent>? _subscription;

  // State
  bool _isListening = false;
  final List<LogEntry> _logs = []; // Use public LogEntry from logs widget
  final ScrollController _scrollController = ScrollController();

  void _addLog(String message, {bool isError = false, bool isLive = false}) {
    if (!mounted) return;
    setState(() {
      _logs.insert(
        0,
        LogEntry(
          message: message,
          timestamp: DateTime.now(),
          isError: isError,
          isLive: isLive,
        ),
      );
    });
  }

  String? _activeQueryUuid;

  Future<void> _toggleListener() async {
    if (_isListening) {
      await _stopListening();
    } else {
      _startListening();
    }
  }

  void _startListening() {
    final tableName = _tableController.text.trim();
    if (tableName.isEmpty) {
      _addLog("Table name cannot be empty", isError: true);
      return;
    }

    try {
      _addLog("Initializing stream for '$tableName'...");

      final stream = widget.controller.client!.local!.getClient.liveQuery(
        tableName: tableName,
        snapshot: true,
      );

      _subscription = stream.listen(
        (event) {
          // TERMINAL LOGGING FOR DEBUGGING
          print(
            "🔍 DART LIVE QUERY DEBUG: Action: ${event.action}, Result: ${event.result}, UUID: ${event.queryUuid}",
          );

          if (event.queryUuid != null) {
            _activeQueryUuid = event.queryUuid;
            final msg = "🔌 Handshake Received! Query UUID: $_activeQueryUuid";
            _addLog(msg);
            print(msg);
            return;
          }

          final logMsg =
              "🟢 LIVE EVENT: Action: ${event.action}, ID: ${event.id}, Data: ${event.result}";
          _addLog(logMsg, isLive: true);
          print(logMsg);
        },
        onError: (e) {
          final msg = "Stream Error: $e";
          _addLog(msg, isError: true);
          print("🛑 $msg");
          _stopListening(force: true); // Auto-stop on error
        },
        onDone: () {
          final msg = "Stream Closed (Done).";
          _addLog(msg);
          print("🏁 $msg");
          _stopListening(force: true);
        },
      );

      setState(() {
        _isListening = true;
      });
      _addLog("Listening to '$tableName'.");
    } catch (e) {
      _addLog("Failed to start stream: $e", isError: true);
    }
  }

  Future<void> _stopListening({bool force = false}) async {
    _addLog("Stopping stream...");
    try {
      await _subscription?.cancel();
      _subscription = null;

      // KILL SWITCH
      // The StreamSubscription wrapper in flutter_surrealdb_engine automatically calls killQuery on cancel.
      // We do NOT need to call it manually here, as that causes double-invocations and potential hangs.
      if (_activeQueryUuid != null) {
        _addLog("🔪 Sending Kill Switch for $_activeQueryUuid...");
        await SurrealDb.killQuery(queryUuid: _activeQueryUuid!);
        _addLog("💀 Query Killed.");
      }
      _activeQueryUuid = null;
    } catch (e) {
      _addLog("Error canceling stream: $e", isError: true);
    } finally {
      if (mounted) {
        setState(() {
          _isListening = false;
        });
        _addLog("Stopped listening.");
      }
    }
  }

  Future<void> _createRecord() async {
    final tableName = _tableController.text;
    final now = DateTime.now().millisecondsSinceEpoch;
    try {
      const String query =
          r'''CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password)''';
      final credentials = {"username": "user$now", "password": "password123"};

      // Use create() directly to match integration test behavior
      final result = await widget.controller.client!.local!.getClient.create(
        resource: 'user',
        data: jsonEncode(credentials),
      );

      final dynamic decoded = jsonDecode(result);
      // SurrealDB create often returns a list of created records
      final id = decoded is List && decoded.isNotEmpty
          ? decoded[0]['id']
          : decoded['id'];
      _addLog("Created User: $id");
    } catch (e) {
      _addLog("Create Failed: $e", isError: true);
    }
  }

  void _clearLogs() {
    setState(() {
      _logs.clear();
    });
  }

  Future<void> _runDiagnostics() async {
    _addLog("--- DIAGNOSTICS START ---");
    try {
      final client = widget.controller.client;
      if (client == null) {
        _addLog("Client is NULL", isError: true);
        return;
      }
      if (client.local == null) {
        _addLog("Local Service is NULL", isError: true);
        return;
      }

      _addLog("Checking DB Connection...");
      final info = await client.local!.getClient.query(sql: "INFO FOR DB");
      _addLog("INFO FOR DB: $info");

      _addLog("Checking 'user' table count...");
      final count = await client.local!.getClient.query(
        sql: "SELECT count() FROM user",
      );
      _addLog("Count: $count");

      _addLog("Checking permissions (Schema)...");
      final schema = await client.local!.getClient.query(
        sql: "INFO FOR TABLE user",
      );
      _addLog("Schema: $schema");

      _addLog("Diagnosis: Backend seems responsive.");
    } catch (e) {
      _addLog("DIAGNOSIS FAILED: $e", isError: true);
    }
    _addLog("--- DIAGNOSTICS END ---");
  }

  @override
  void dispose() {
    _subscription?.cancel();
    _tableController.dispose();
    _scrollController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: SpookyColors.background,
      appBar: AppBar(
        title: Text(
          "[ LIVE QUERY DASHBOARD ]",
          style: GoogleFonts.spaceMono(
            fontWeight: FontWeight.bold,
            fontSize: 16, // Matches SpookyAppBar
            letterSpacing: 1.2, // Matches SpookyAppBar
            color: SpookyColors.white,
          ),
        ),
        backgroundColor: SpookyColors.background,
        iconTheme: const IconThemeData(color: SpookyColors.white),
        elevation: 0,
        centerTitle: true,
        actions: [
          IconButton(
            icon: const Icon(Icons.preview),
            tooltip: "Visual Demo",
            onPressed: () {
              Navigator.of(context).push(
                MaterialPageRoute(
                  builder: (context) =>
                      LiveQueryView(controller: widget.controller),
                ),
              );
            },
          ),
        ],
      ),
      body: Column(
        children: [
          // 1. Controls
          LiveQueryControls(
            tableController: _tableController,
            isListening: _isListening,
            onToggleListener: _toggleListener,
            onCreateRecord: _createRecord,
          ),
          ElevatedButton.icon(
            onPressed: _runDiagnostics,
            icon: const Icon(Icons.bug_report),
            label: const Text("Run Diagnostics"),
            style: ElevatedButton.styleFrom(backgroundColor: Colors.orange),
          ),

          const Divider(height: 1, color: SpookyColors.white10),

          // 2. Log Console
          Expanded(
            child: LiveQueryLogs(
              logs: _logs,
              scrollController: _scrollController,
              onClear: _clearLogs,
            ),
          ),
        ],
      ),
    );
  }
}
