import 'dart:convert';
import 'dart:async';
import 'package:flutter/material.dart';
import '../../controllers/spooky_controller.dart';
import '../../components/action_card.dart';

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
      final stream = widget.controller.client!.remote.getClient.liveQuery(
        tableName: tableName,
      );
      setState(() {
        _isListening = true;
        _events.add("[Local] Subscribing to '$tableName'...");
      });

      _subscription = stream.listen(
        (eventJson) {
          // Parse prettily if possible
          try {
            final decoded = jsonDecode(eventJson);
            final pretty = const JsonEncoder.withIndent('  ').convert(decoded);
            setState(() {
              _events.insert(0, "[Event] $pretty");
            });
          } catch (_) {
            setState(() {
              _events.insert(0, "[Raw] $eventJson");
            });
          }
        },
        onError: (err) {
          setState(() {
            _events.add("[Error] Stream error: $err");
            _isListening = false;
          });
        },
        onDone: () {
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
      final data = tableName == 'user'
          ? {
              "username": "user_$now",
              "password": "password123",
              /* required by schema */
              "created_at": DateTime.now().toIso8601String(),
            }
          : {
              "name": "Test Item $now",
              "created_at": DateTime.now().toIso8601String(),
            };

      final result = await widget.controller.client!.remote.getClient.create(
        resource: tableName,
        data: jsonEncode(data),
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

      final result = await widget.controller.client!.remote.getClient.query(
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

  @override
  void dispose() {
    _subscription?.cancel();
    _tableController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Live Query Test")),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.all(16.0),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _tableController,
                    decoration: const InputDecoration(labelText: "Table Name"),
                  ),
                ),
                const SizedBox(width: 16),
                ElevatedButton(
                  onPressed: _toggleSubscription,
                  style: ElevatedButton.styleFrom(
                    backgroundColor: _isListening ? Colors.red : Colors.green,
                  ),
                  child: Text(
                    _isListening ? "Stop Listening" : "Start Listening",
                  ),
                ),
              ],
            ),
          ),
          Wrap(
            spacing: 16,
            children: [
              ElevatedButton.icon(
                onPressed: _createRecord,
                icon: const Icon(Icons.add),
                label: const Text("Create Record"),
              ),
              ElevatedButton.icon(
                onPressed: _updateRecord,
                icon: const Icon(Icons.update),
                label: const Text("Update All"),
              ),
            ],
          ),
          const Divider(),
          Expanded(
            child: ListView.builder(
              itemCount: _events.length,
              itemBuilder: (context, index) {
                final text = _events[index];
                return Container(
                  padding: const EdgeInsets.all(8),
                  color: index % 2 == 0 ? Colors.black12 : Colors.transparent,
                  child: Text(
                    text,
                    style: const TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 12,
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
