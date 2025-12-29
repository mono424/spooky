import 'dart:convert';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../../controllers/spooky_controller.dart';
import 'widgets/live_query_input.dart';
import 'widgets/live_query_actions.dart';
import 'widgets/live_query_logs.dart';

class LiveQueryView extends StatefulWidget {
  final SpookyController controller;

  const LiveQueryView({super.key, required this.controller});

  @override
  State<LiveQueryView> createState() => _LiveQueryViewState();
}

class _LiveQueryViewState extends State<LiveQueryView> {
  final TextEditingController _tableController = TextEditingController(
    text: 'user',
  );
  final ScrollController _logScrollController = ScrollController();

  StreamSubscription<List<Map<String, dynamic>>>? _subscription;
  final List<LogEntry> _logs = []; // Changed to LogEntry
  bool _isListening = false;

  @override
  void dispose() {
    _subscription?.cancel();
    _tableController.dispose();
    _logScrollController.dispose();
    super.dispose();
  }

  void _addLog(String message, {bool isError = false, bool isLive = false}) {
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

  void _clearLogs() {
    setState(() {
      _logs.clear();
    });
  }

  void _toggleSubscription() async {
    if (_isListening) {
      try {
        await _subscription?.cancel();
        _subscription = null;
        setState(() {
          _isListening = false;
        });
        _addLog("[Local] Subscription cancelled.");
      } catch (e) {
        setState(() {
          _isListening = false; // Force state update
          _subscription = null;
        });
        _addLog("[Error] Cancel failed (forced stop): $e", isError: true);
      }
      return;
    }

    final tableName = _tableController.text;
    if (tableName.isEmpty) return;

    try {
      // Use High-Level API
      final client = widget.controller.client;
      if (client == null) throw Exception("Client is null");

      final stream = client.local.getClient
          .select(resource: tableName)
          .live((json) => json); // Return the map directly

      setState(() {
        _isListening = true;
      });
      _addLog("[Local] Subscribing to '$tableName' (High-Level)...");
      debugPrint("LiveQueryView: Subscribing to $tableName");

      _subscription = stream.listen(
        (items) {
          debugPrint("LiveQueryView: Received update: ${items.length} items");
          final pretty = const JsonEncoder.withIndent('  ').convert(items);
          _addLog("[LIVE STATE] ${items.length} items:\n$pretty", isLive: true);
        },
        onError: (err) {
          debugPrint("LiveQueryView: Stream error: $err");
          _addLog("[Error] Stream error: $err", isError: true);
          setState(() {
            _isListening = false;
          });
        },
        onDone: () {
          debugPrint("LiveQueryView: Stream closed (onDone)");
          _addLog("[Done] Stream closed.");
          setState(() {
            _isListening = false;
          });
        },
      );
    } catch (e) {
      _addLog("[Error] Failed to start live query: $e", isError: true);
    }
  }

  Future<void> _createRecord() async {
    try {
      final now = DateTime.now().millisecondsSinceEpoch;

      // Adapted for 'user' table schema
      const String query =
          r'''CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password)''';

      final credentials = {"username": "user$now", "password": "password123"};

      final client = widget.controller.client;
      if (client == null) throw Exception("Client is null");

      final result = await client.local.getClient.query(
        sql: query,
        vars: jsonEncode(credentials),
      );

      if (result.contains("ERR") || result.contains("error")) {
        throw Exception("DB Error: $result");
      }

      _addLog("[Success] Created: $result");
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Create Error: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
      _addLog("[Error] Create failed: $e", isError: true);
    }
  }

  Future<void> _updateRecord() async {
    try {
      final tableName = _tableController.text;
      final now = DateTime.now().millisecondsSinceEpoch;

      // Adapted for 'user' table which doesn't have updated_at
      final updateQuery = tableName == 'user'
          ? "UPDATE type::table(\$tb) MERGE { password: \$val };"
          : "UPDATE type::table(\$tb) MERGE { updated_at: \$val };";

      final val = tableName == 'user'
          ? "pass_$now"
          : DateTime.now().toIso8601String();

      final client = widget.controller.client;
      if (client == null) throw Exception("Client is null");

      final result = await client.local.getClient.query(
        sql: updateQuery,
        vars: jsonEncode({"tb": tableName, "val": val}),
      );

      if (result.contains("ERR") || result.contains("error")) {
        throw Exception("DB Error: $result");
      }

      _addLog("[Success] Update All: $result");
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Update Error: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
      _addLog("[Error] Update failed: $e", isError: true);
    }
  }

  Future<void> _copyAllLogs() async {
    final allText = _logs.map((e) => "${e.timestamp}: ${e.message}").join('\n');
    await Clipboard.setData(ClipboardData(text: allText));
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("All logs copied to clipboard")),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text("Live Query Test (High Level)"),
        actions: [
          IconButton(
            icon: const Icon(Icons.copy),
            tooltip: "Copy All Logs",
            onPressed: _copyAllLogs,
          ),
        ],
      ),
      body: Column(
        children: [
          LiveQueryInput(
            controller: _tableController,
            isListening: _isListening,
            onToggle: _toggleSubscription,
          ),
          LiveQueryActions(onCreate: _createRecord, onUpdate: _updateRecord),
          const Divider(),
          Expanded(
            child: LiveQueryLogs(
              logs: _logs,
              scrollController: _logScrollController,
              onClear: _clearLogs,
            ),
          ),
        ],
      ),
    );
  }
}
