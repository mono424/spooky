import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import '../../controllers/spooky_controller.dart';
import '../../components/action_card.dart';

class UpsyncTestView extends StatefulWidget {
  final SpookyController controller;

  const UpsyncTestView({super.key, required this.controller});

  @override
  State<UpsyncTestView> createState() => _UpsyncTestViewState();
}

class _UpsyncTestViewState extends State<UpsyncTestView> {
  final TextEditingController _idController = TextEditingController(
    text: 'user:test_1',
  );
  final TextEditingController _dataController = TextEditingController(
    text: '''{
  "username": "test_user",
  "password": "password123"
}''',
  );

  String _status = "Ready";

  Future<void> _create() async {
    setState(() => _status = "Creating...");
    try {
      final id = _idController.text;
      final data = jsonDecode(_dataController.text) as Map<String, dynamic>;

      // We assume table is inferred from ID or passed.
      // SpookyClient.create(id, data) requires full ID.

      final res = await widget.controller.create(id, data);
      setState(
        () => _status =
            "Created Mutation: ${res.mutationID}, Target: ${res.target?.id}",
      );
      widget.controller.log(
        "Created local record: ${res.target?.id} (Mutation: ${res.mutationID})",
      );
    } catch (e) {
      setState(() => _status = "Error: $e");
      widget.controller.log("Error creating record: $e");
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Upsync Test")),
      body: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          children: [
            TextField(
              controller: _idController,
              decoration: const InputDecoration(
                labelText: "Record ID (table:id)",
              ),
            ),
            const SizedBox(height: 10),
            TextField(
              controller: _dataController,
              decoration: const InputDecoration(labelText: "JSON Data"),
              maxLines: 5,
            ),
            const SizedBox(height: 20),
            ElevatedButton(
              onPressed: _create,
              child: const Text("Create (Local -> Sync)"),
            ),
            const SizedBox(height: 10),
            ElevatedButton(
              onPressed: () async {
                setState(() => _status = "Testing 8666...");
                try {
                  // Direct connection test using the engine
                  final client = await SurrealDb.connect(
                    mode: StorageMode.remote(url: 'ws://127.0.0.1:8666/rpc'),
                  );
                  // Optional: Authenticate or Query to ensure protocol handshake finished
                  // await client.authenticate(token: ...);
                  await client.close();
                  setState(() => _status = "Success 8666: Connected");
                  widget.controller.log("Test 8666: Connected OK.");
                } catch (e) {
                  setState(() => _status = "Fail 8666: $e");
                  widget.controller.log("Test 8666 Failed: $e");
                }
              },
              child: const Text("Test Port 8666 (v2.1.2?)"),
            ),
            const SizedBox(height: 10),
            ElevatedButton(
              onPressed: () async {
                setState(() => _status = "Testing 8999 (v3)...");
                try {
                  final client = await SurrealDb.connect(
                    // Connect to the temporary v3 test server
                    mode: StorageMode.remote(url: 'ws://127.0.0.1:8999/rpc'),
                  );
                  await client.close();
                  setState(() => _status = "Success 8999 (v3): Connected");
                  widget.controller.log("Test 8999 (v3): Connected OK.");
                } catch (e) {
                  setState(() => _status = "Fail 8999: $e");
                  widget.controller.log("Test 8999 Failed: $e");
                }
              },
              child: const Text("Test Port 8999 (v3.0.0-beta.1)"),
            ),
            const SizedBox(height: 20),
            Text("Status: $_status"),
          ],
        ),
      ),
    );
  }
}
