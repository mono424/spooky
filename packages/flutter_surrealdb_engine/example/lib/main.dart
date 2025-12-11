import 'dart:io';

import 'package:flutter/material.dart';
import 'dart:async';
import 'package:path_provider/path_provider.dart';

import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

void main() {
  runApp(const MyApp());
}

class MyApp extends StatefulWidget {
  const MyApp({super.key});

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  String _status = 'Not connected';
  final List<String> _logs = [];

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    try {
      await RustLib.init();
      setState(() {
        _status = 'Rust initialized';
      });
    } catch (e) {
      setState(() {
        _status = 'Initialization error: $e';
      });
    }
  }

  Future<void> _connect() async {
    try {
      // Connect to a local file database
      final Directory directory = await getApplicationDocumentsDirectory();
      await connectDb(
        path:
            '~/projekts/spooky/packages/flutter_surrealdb_engine/example/surreal.db',
      );
      setState(() {
        _status = 'Connected to surreal.db';
        _logs.add('Connected successfully');
      });
    } catch (e) {
      setState(() {
        _status = 'Connection error: $e';
        _logs.add('Connection error: $e');
      });
    }
  }

  Future<void> _createRecord() async {
    try {
      // Create a new record in the 'person' table
      final query =
          "CREATE person CONTENT { name: 'Timothy Besel', age: 28, created_at: time::now() };";
      final results = await queryDb(query: query);

      setState(() {
        for (var res in results) {
          _logs.add('Status: ${res.status}, Time: ${res.time}');
          if (res.result != null) {
            _logs.add('Result: ${res.result}');
          }
        }
      });
    } catch (e) {
      setState(() {
        _logs.add('Query error: $e');
      });
    }
  }

  Future<void> _queryRecords() async {
    try {
      final results = await queryDb(query: "SELECT * FROM person;");
      setState(() {
        for (var res in results) {
          _logs.add('Query Status: ${res.status}');
          if (res.result != null) {
            _logs.add('Records: ${res.result}');
          }
        }
      });
    } catch (e) {
      setState(() {
        _logs.add('Select error: $e');
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('SurrealDB Engine Example')),
        body: Column(
          children: [
            Padding(
              padding: const EdgeInsets.all(8.0),
              child: Text('Status: $_status'),
            ),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceEvenly,
              children: [
                ElevatedButton(
                  onPressed: _connect,
                  child: const Text('Connect DB'),
                ),
                ElevatedButton(
                  onPressed: _createRecord,
                  child: const Text('Create Record'),
                ),
                ElevatedButton(
                  onPressed: _queryRecords,
                  child: const Text('Query All'),
                ),
              ],
            ),
            Expanded(
              child: ListView.builder(
                itemCount: _logs.length,
                itemBuilder: (context, index) {
                  return ListTile(title: Text(_logs[index]), dense: true);
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
