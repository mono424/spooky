import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../../controllers/spooky_controller.dart';
import '../../controllers/spooky_controller.dart';
import '../../core/theme.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart'; // Import for SurrealDb static methods

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
  final List<_LogEntry> _logs = [];
  final ScrollController _scrollController = ScrollController();

  void _addLog(String message, {bool isError = false, bool isLive = false}) {
    if (!mounted) return;
    setState(() {
      _logs.insert(
        0,
        _LogEntry(
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

      final result = await widget.controller.client!.local!.getClient.query(
        sql: query,
        vars: jsonEncode(credentials),
      );

      if (result.contains("ERR") || result.contains("error")) {
        throw Exception(result);
      }
      _addLog("Created User: user$now");
    } catch (e) {
      _addLog("Create Failed: $e", isError: true);
    }
  }

  void _clearLogs() {
    setState(() {
      _logs.clear();
    });
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
          "Live Query Dashboard",
          style: GoogleFonts.spaceMono(
            fontWeight: FontWeight.bold,
            color: SpookyColors.white,
          ),
        ),
        backgroundColor: SpookyColors.background,
        iconTheme: const IconThemeData(color: SpookyColors.white),
        elevation: 0,
        actions: [
          IconButton(
            icon: const Icon(Icons.delete_sweep, color: SpookyColors.white60),
            onPressed: _clearLogs,
            tooltip: "Clear Logs",
          ),
        ],
      ),
      body: Column(
        children: [
          // 1. Control Panel
          Container(
            padding: const EdgeInsets.all(16),
            color: SpookyColors.surface,
            child: Column(
              children: [
                Row(
                  children: [
                    Expanded(
                      child: TextField(
                        controller: _tableController,
                        style: GoogleFonts.spaceMono(color: SpookyColors.white),
                        decoration: InputDecoration(
                          labelText: "Target Table",
                          labelStyle: GoogleFonts.spaceMono(
                            color: SpookyColors.white60,
                          ),
                          prefixIcon: const Icon(
                            Icons.table_chart,
                            color: SpookyColors.primary,
                          ),
                          enabledBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.zero,
                            borderSide: BorderSide(
                              color: SpookyColors.white.withOpacity(0.1),
                            ),
                          ),
                          focusedBorder: const OutlineInputBorder(
                            borderRadius: BorderRadius.zero,
                            borderSide: BorderSide(color: SpookyColors.primary),
                          ),
                          isDense: true,
                          enabled: !_isListening, // Lock input while listening
                          fillColor: _isListening
                              ? SpookyColors.white.withOpacity(0.05)
                              : null,
                          filled: _isListening,
                        ),
                      ),
                    ),
                    const SizedBox(width: 16),
                    _StatusIndicator(isListening: _isListening),
                  ],
                ),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(
                      child: ElevatedButton.icon(
                        onPressed: _toggleListener,
                        style: ElevatedButton.styleFrom(
                          shape: const RoundedRectangleBorder(
                            borderRadius: BorderRadius.zero,
                          ),
                          backgroundColor: _isListening
                              ? Colors.redAccent.withOpacity(0.1)
                              : Colors.greenAccent.withOpacity(0.1),
                          foregroundColor: _isListening
                              ? Colors.redAccent
                              : Colors.greenAccent,
                          padding: const EdgeInsets.symmetric(vertical: 16),
                          side: BorderSide(
                            color: _isListening
                                ? Colors.redAccent
                                : Colors.greenAccent,
                          ),
                        ),
                        icon: Icon(
                          _isListening ? Icons.stop : Icons.play_arrow,
                        ),
                        label: Text(
                          _isListening
                              ? "STOP SUBSCRIPTION"
                              : "START SUBSCRIPTION",
                          style: GoogleFonts.spaceMono(
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
                SizedBox(
                  width: double.infinity,
                  child: OutlinedButton.icon(
                    onPressed: _createRecord,
                    style: OutlinedButton.styleFrom(
                      shape: const RoundedRectangleBorder(
                        borderRadius: BorderRadius.zero,
                      ),
                      foregroundColor: SpookyColors.white,
                      side: const BorderSide(
                        color: Colors.white, // Explicit white as requested
                        width: 1.0,
                      ),
                      padding: const EdgeInsets.symmetric(vertical: 16),
                    ),
                    icon: const Icon(Icons.add_circle_outline),
                    label: Text(
                      "Create Test Record",
                      style: GoogleFonts.spaceMono(),
                    ),
                  ),
                ),
              ],
            ),
          ),
          const Divider(height: 1, color: SpookyColors.white10),
          // 2. Log Console
          Expanded(
            child: Container(
              color: const Color(0xFF0A0A0A), // Even darker for console
              child: ListView.separated(
                controller: _scrollController,
                padding: const EdgeInsets.all(12),
                itemCount: _logs.length,
                separatorBuilder: (_, __) => Divider(
                  color: SpookyColors.white.withOpacity(0.05),
                  height: 1,
                ),
                itemBuilder: (context, index) {
                  final log = _logs[index];
                  return _LogItem(log: log);
                },
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _StatusIndicator extends StatelessWidget {
  final bool isListening;
  const _StatusIndicator({required this.isListening});

  @override
  Widget build(BuildContext context) {
    final color = isListening ? Colors.greenAccent : Colors.grey;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: color.withOpacity(0.1),
        borderRadius: BorderRadius.zero, // Sharp edges
        border: Border.all(color: color.withOpacity(0.5), width: 1),
      ),
      child: Row(
        children: [
          Icon(
            isListening ? Icons.sensors : Icons.sensors_off,
            color: color,
            size: 16,
          ),
          const SizedBox(width: 8),
          Text(
            isListening ? "ACTIVE" : "IDLE",
            style: GoogleFonts.spaceMono(
              fontWeight: FontWeight.bold,
              color: color,
            ),
          ),
        ],
      ),
    );
  }
}

class _LogEntry {
  final String message;
  final DateTime timestamp;
  final bool isError;
  final bool isLive;

  _LogEntry({
    required this.message,
    required this.timestamp,
    this.isError = false,
    this.isLive = false,
  });
}

class _LogItem extends StatelessWidget {
  final _LogEntry log;

  const _LogItem({required this.log});

  @override
  Widget build(BuildContext context) {
    Color textColor = SpookyColors.white.withOpacity(0.7);
    if (log.isError) textColor = Colors.redAccent;
    if (log.isLive) textColor = Colors.greenAccent;

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            "${log.timestamp.hour.toString().padLeft(2, '0')}:${log.timestamp.minute.toString().padLeft(2, '0')}:${log.timestamp.second.toString().padLeft(2, '0')}",
            style: GoogleFonts.spaceMono(
              color: SpookyColors.white.withOpacity(0.3),
              fontSize: 10,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: SelectableText(
              log.message,
              style: GoogleFonts.spaceMono(color: textColor, fontSize: 12),
            ),
          ),
        ],
      ),
    );
  }
}
