import 'dart:convert';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../../controllers/spooky_controller.dart';
import '../../components/action_card.dart';
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
  StreamSubscription<String>? _subscription;
  final List<String> _events = [];
  bool _isListening = false;

  void _toggleSubscription() async {
    if (_isListening) {
      try {
        await _subscription?.cancel();
        _subscription = null;
        setState(() {
          _isListening = false;
          _events.add("[Local] Subscription cancelled.");
        });
      } catch (e) {
        setState(() {
          _isListening = false; // Force state update
          _subscription = null;
          _events.add("[Error] Cancel failed (forced stop): $e");
        });
      }
      return;
    }

    final tableName = _tableController.text;
    if (tableName.isEmpty) return;

    try {
      final stream = widget.controller.client!.local!.getClient.liveQuery(
        tableName: tableName,
      );
      setState(() {
        _isListening = true;
        _events.add("[Local] Subscribing to '$tableName'...");
        debugPrint("LiveQueryView: Subscribing to $tableName");
      });

      _subscription = stream.listen(
        (eventJson) {
          debugPrint("LiveQueryView: Received event: $eventJson");
          // Parse prettily if possible
          try {
            final decoded = jsonDecode(eventJson);
            final pretty = const JsonEncoder.withIndent('  ').convert(decoded);
            setState(() {
              _events.insert(0, "[LIVE] $pretty");
            });
          } catch (_) {
            setState(() {
              _events.insert(0, "[LIVE RAW] $eventJson");
            });
          }
        },
        onError: (err) {
          debugPrint("LiveQueryView: Stream error: $err");
          setState(() {
            _events.add("[Error] Stream error: $err");
            _isListening = false;
          });
        },
        onDone: () {
          debugPrint("LiveQueryView: Stream closed (onDone)");
          setState(() {
            _events.add("[Done] Stream closed.");
            _isListening = false;
          });
        },
      );
    } catch (e) {
      setState(() {
        _events.add("[Error] Failed to start live query: $e");
      });
    }
  }

  Future<void> _createRecord() async {
    try {
      final tableName = _tableController.text;
      final now = DateTime.now().millisecondsSinceEpoch;

      // Adapted for 'user' table schema
      const String query =
          r'''CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password)''';

      final credentials = {"username": "user$now", "password": "password123"};

      final result = await widget.controller.client!.local!.getClient.query(
        sql: query,
        vars: jsonEncode(credentials),
      );

      if (result.contains("ERR") || result.contains("error")) {
        throw Exception("DB Error: $result");
      }

      setState(() {
        _events.insert(0, "[Success] Created: $result");
      });
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Create Error: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
      setState(() {
        _events.add("[Error] Create failed: $e");
      });
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

      final result = await widget.controller.client!.local!.getClient.query(
        sql: updateQuery,
        vars: jsonEncode({"tb": tableName, "val": val}),
      );

      if (result.contains("ERR") || result.contains("error")) {
        throw Exception("DB Error: $result");
      }

      setState(() {
        _events.insert(0, "[Success] Update All: $result");
      });
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text("Update Error: $e"),
            backgroundColor: Colors.red,
          ),
        );
      }
      setState(() {
        _events.add("[Error] Update failed: $e");
      });
    }
  }

  Future<void> _copyAllLogs() async {
    final allText = _events.join('\n');
    await Clipboard.setData(ClipboardData(text: allText));
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("All logs copied to clipboard")),
      );
    }
  }

  @override
  void dispose() {
    _subscription?.cancel();
    _tableController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text("Live Query Test"),
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
          Expanded(child: LiveQueryLogs(events: _events)),
        ],
      ),
    );
  }
}
