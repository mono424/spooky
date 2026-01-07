import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart'; // Import for RecordId
import '../../controllers/spooky_controller.dart';
import '../../components/action_card.dart';

class UpsyncTestView extends StatefulWidget {
  final SpookyController controller;

  const UpsyncTestView({super.key, required this.controller});

  @override
  State<UpsyncTestView> createState() => _UpsyncTestViewState();
}

class _UpsyncTestViewState extends State<UpsyncTestView> {
  // We'll generate IDs dynamically for the flow
  String _status = "Ready";

  Future<void> _createThreadAndComment() async {
    setState(() => _status = "Starting Flow...");
    try {
      final threadIdVal = 'thread:test_${DateTime.now().millisecondsSinceEpoch}';
      final commentIdVal = 'comment:test_${DateTime.now().millisecondsSinceEpoch}';
      // Use authenticated user or fallback to provided ID
      final userIdVal = widget.controller.userId ?? 'user:rvkme6hk9ckgji6dlcvx';

      // 1. Create Thread
      setState(() => _status = "Creating Thread $threadIdVal...");
      final threadRes = await widget.controller.create(
        RecordId.fromString(threadIdVal),
        {
          'title': 'Test Thread from Upsync',
          'content': 'This thread was created via UpsyncTestView flow.',
          'author': RecordId.fromString(userIdVal),
          'active': true,
        },
      );
      widget.controller.log("Created Thread: ${threadRes.target?.id}");

      // 2. Create Comment
      setState(() => _status = "Creating Comment linked to Thread...");
      final commentRes = await widget.controller.create(
        RecordId.fromString(commentIdVal),
        {
          'content': 'This is a comment on the test thread.',
          'thread': RecordId.fromString(threadIdVal), // LINK TO THREAD
          'author': RecordId.fromString(userIdVal),
        },
      );
      widget.controller.log("Created Comment: ${commentRes.target?.id}");

      setState(() => _status = "Success! Created Thread & Comment.");
      
    } catch (e) {
      setState(() => _status = "Error: $e");
      widget.controller.log("Error during flow: $e");
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Upsync Test")),
      body: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const Text(
                "Test Sync Flow",
                style: TextStyle(fontSize: 20, fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 10),
              const Text("Creates a Thread -> Then creates a Comment linked to it."),
              const SizedBox(height: 30),
              
              ElevatedButton(
                onPressed: _createThreadAndComment,
                style: ElevatedButton.styleFrom(
                  padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 16),
                ),
                child: const Text("Run Creation Flow"),
              ),
              
              const SizedBox(height: 20),
              Text(
                "Status: $_status",
                textAlign: TextAlign.center,
                style: const TextStyle(color: Colors.grey),
              ),

              const Divider(height: 50),

              // Network utilities
               ElevatedButton(
                onPressed: () async {
                  setState(() => _status = "Testing 8666...");
                  try {
                    final client = await SurrealDb.connect(
                      mode: StorageMode.remote(url: 'ws://127.0.0.1:8666/rpc'),
                    );
                    await client.close();
                    setState(() => _status = "Success 8666: Connected");
                  } catch (e) {
                    setState(() => _status = "Fail 8666: $e");
                  }
                },
                child: const Text("Test Port 8666"),
              ),
              const SizedBox(height: 10),
               ElevatedButton(
                onPressed: () async {
                  setState(() => _status = "Clearing Queue...");
                  try {
                    // Force clear the local pending mutations table
                    await widget.controller.client!.local.query('DELETE _spooky_pending_mutations');
                    setState(() => _status = "Queue Cleared!");
                    widget.controller.log("Cleared _spooky_pending_mutations.");
                  } catch (e) {
                    setState(() => _status = "Clear Failed: $e");
                  }
                },
                style: ElevatedButton.styleFrom(backgroundColor: Colors.red.withValues(alpha: 0.2)),
                 child: const Text("Clear Pending Queue (Fix Stuck Sync)"),
               ),
              const SizedBox(height: 10),
               ElevatedButton(
                onPressed: () async {
                  setState(() => _status = "Testing 8999...");
                  try {
                    final client = await SurrealDb.connect(
                      mode: StorageMode.remote(url: 'ws://127.0.0.1:8999/rpc'),
                    );
                    await client.close();
                    setState(() => _status = "Success 8999: Connected");
                  } catch (e) {
                    setState(() => _status = "Fail 8999: $e");
                  }
                },
                child: const Text("Test Port 8999"),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
