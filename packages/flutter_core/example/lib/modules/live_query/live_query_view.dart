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
    text: 'person',
  );
  StreamSubscription<String>? _subscription;
  final List<String> _events = [];
  bool _isListening = false;

  void _toggleSubscription() async {
    if (_isListening) {
      await _subscription?.cancel();
      setState(() {
        _isListening = false;
        _subscription = null;
        _events.add("[Local] Subscription cancelled.");
      });
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
      await widget.controller.client!.remote.getClient.create(
        resource: tableName,
        data: jsonEncode({
          "name": "Test User",
          "created_at": DateTime.now().toIso8601String(),
        }),
      );
      // Log happens automatically via stream if working
    } catch (e) {
      setState(() {
        _events.add("[Error] Create failed: $e");
      });
    }
  }

  Future<void> _updateRecord() async {
    // This is hard to do without an ID, so we might just create another one
    // Or we update ALL records in table (dangerous but effective for test)
    try {
      final tableName = _tableController.text;
      // MERGE to all records in table
      await widget.controller.client!.remote.getClient.query(
        sql: "UPDATE type::table(\$tb) MERGE { updated_at: \$now };",
        vars: jsonEncode({
          "tb": tableName,
          "now": DateTime.now().toIso8601String(),
        }),
      );
    } catch (e) {
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
