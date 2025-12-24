import 'dart:convert';
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
  SurrealDatabase? _db;

  // Inputs
  final TextEditingController _pathController = TextEditingController();
  final TextEditingController _nsController = TextEditingController(
    text: 'test_ns',
  );
  final TextEditingController _dbController = TextEditingController(
    text: 'test_db',
  );
  final TextEditingController _userController = TextEditingController(
    text: 'root',
  );
  final TextEditingController _passController = TextEditingController(
    text: 'root',
  );
  final TextEditingController _tokenController = TextEditingController();
  final TextEditingController _queryController = TextEditingController(
    text: 'SELECT * FROM person',
  );

  @override
  void initState() {
    super.initState();
    _init();
  }

  Future<void> _init() async {
    try {
      await RustLib.init();
      final dir = await getApplicationDocumentsDirectory();
      _pathController.text = '${dir.path}/surreal.db';
      setState(() {
        _status = 'Rust initialized';
      });
    } catch (e) {
      setState(() {
        _status = 'Initialization error: $e';
      });
    }
  }

  void _log(String message) {
    setState(() {
      _logs.insert(0, message);
    });
  }

  Future<void> _connect() async {
    try {
      _db = await connectDb(path: _pathController.text);
      setState(() {
        _status = 'Connected';
      });
      _log('Connected to ${_pathController.text}');
    } catch (e) {
      _log('Connection error: $e');
    }
  }

  Future<void> _health() async {
    if (_db == null) return;
    try {
      final res = await _db!.health();
      _log('Health: ${res.first.status} (${res.first.time})');
    } catch (e) {
      _log('Health error: $e');
    }
  }

  Future<void> _version() async {
    if (_db == null) return;
    try {
      final res = await _db!.version();
      _log('Version: ${res.first.result} (${res.first.time})');
    } catch (e) {
      _log('Version error: $e');
    }
  }

  Future<void> _signinRoot() async {
    if (_db == null) return;
    try {
      // Ensure user exists first (for embedded)
      await _db!.queryDb(
        query:
            "DEFINE USER ${_userController.text} ON ROOT PASSWORD '${_passController.text}' ROLES OWNER;",
      );

      final res = await _db!.signinRoot(
        username: _userController.text,
        password: _passController.text,
      );
      if (res.isNotEmpty && res.first.result != null) {
        final rawJson = res.first.result!;
        // Parse the JSON string to get the raw token if it's a string
        try {
          final decoded = jsonDecode(rawJson);
          _tokenController.text = decoded is String ? decoded : rawJson;
        } catch (_) {
          _tokenController.text = rawJson;
        }
        _log('Signed in Root: ${res.first.status} (${res.first.time})');
      }
    } catch (e) {
      _log('Signin Root error: $e');
    }
  }

  Future<void> _authenticate() async {
    if (_db == null) return;
    try {
      final res = await _db!.authenticate(token: _tokenController.text);
      _log('Authenticated: ${res.first.status} (${res.first.time})');
    } catch (e) {
      _log('Auth error: $e');
    }
  }

  Future<void> _invalidate() async {
    if (_db == null) return;
    try {
      final res = await _db!.invalidate();
      _log('Invalidated: ${res.first.status} (${res.first.time})');
    } catch (e) {
      _log('Invalidate error: $e');
    }
  }

  Future<void> _useNsDb() async {
    if (_db == null) return;
    try {
      await _db!.useNs(ns: _nsController.text);
      await _db!.useDb(db: _dbController.text);
      _log('Selected NS: ${_nsController.text}, DB: ${_dbController.text}');
    } catch (e) {
      _log('Use NS/DB error: $e');
    }
  }

  Future<void> _createPerson() async {
    if (_db == null) return;
    try {
      final data =
          "{ \"name\": \"User ${DateTime.now().second}\", \"age\": ${DateTime.now().second} }";
      final query = "CREATE person CONTENT $data";
      final res = await _db!.queryDb(query: query);
      if (res.isNotEmpty) {
        _log('Created: ${res.first.result} (${res.first.time})');
      }
    } catch (e) {
      _log('Create error: $e');
    }
  }

  Future<void> _executeQuery() async {
    if (_db == null) return;
    try {
      final res = await _db!.queryDb(query: _queryController.text);
      for (var r in res) {
        _log('Result: ${r.result} (${r.time})');
        if (r.status != "OK") {
          _log('Status: ${r.status}');
        }
      }
    } catch (e) {
      _log('Query error: $e');
    }
  }

  Future<void> _testTransactionCommit() async {
    if (_db == null) return;
    try {
      _log('--- Starting Commit Test ---');
      final tx = await _db!.beginTransaction();
      _log('Transaction started');

      await tx.query(query: "CREATE person:tx_commit SET name = 'Commit Test'");
      _log('Created person:tx_commit inside TX');

      // Verify inside TX
      final resInside = await tx.query(query: "SELECT * FROM person:tx_commit");
      _log('Inside TX Query: ${resInside.first.result}');

      // Verify outside TX (should be empty if isolated, but our current impl might be "read uncommitted" depending on backend)
      // Actually, my Rust test showed full isolation.
      final resOutsideBefore =
          await _db!.queryDb(query: "SELECT * FROM person:tx_commit");
      _log('Outside TX (Before Commit): ${resOutsideBefore.first.result}');

      await tx.commit();
      _log('Transaction committed');

      final resOutsideAfter =
          await _db!.queryDb(query: "SELECT * FROM person:tx_commit");
      _log('Outside TX (After Commit): ${resOutsideAfter.first.result}');
    } catch (e) {
      _log('Transaction Commit Error: $e');
    }
  }

  Future<void> _testTransactionCancel() async {
    if (_db == null) return;
    try {
      _log('--- Starting Cancel Test ---');
      final tx = await _db!.beginTransaction();
      _log('Transaction started');

      await tx.query(query: "CREATE person:tx_cancel SET name = 'Cancel Test'");
      _log('Created person:tx_cancel inside TX');

      await tx.cancel();
      _log('Transaction cancelled');

      final resOutside =
          await _db!.queryDb(query: "SELECT * FROM person:tx_cancel");
      _log('Outside TX (After Cancel): ${resOutside.first.result}');
    } catch (e) {
      _log('Transaction Cancel Error: $e');
    }
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('SurrealDB Engine Demo')),
        body: Column(
          children: [
            Container(
              padding: const EdgeInsets.all(8.0),
              color: Colors.grey[200],
              child: Row(
                children: [
                  const Text(
                    'Status: ',
                    style: TextStyle(fontWeight: FontWeight.bold),
                  ),
                  Text(_status),
                ],
              ),
            ),
            Expanded(
              child: ListView(
                children: [
                  _buildSection('Connection', [
                    TextField(
                      controller: _pathController,
                      decoration: const InputDecoration(
                        labelText: 'Database Path',
                      ),
                    ),
                    ElevatedButton(
                      onPressed: _connect,
                      child: const Text('Connect'),
                    ),
                  ]),
                  _buildSection('General', [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                      children: [
                        ElevatedButton(
                          onPressed: _health,
                          child: const Text('Health'),
                        ),
                        ElevatedButton(
                          onPressed: _version,
                          child: const Text('Version'),
                        ),
                      ],
                    ),
                  ]),
                  _buildSection('Authentication', [
                    TextField(
                      controller: _userController,
                      decoration: const InputDecoration(labelText: 'Username'),
                    ),
                    TextField(
                      controller: _passController,
                      decoration: const InputDecoration(labelText: 'Password'),
                      obscureText: true,
                    ),
                    ElevatedButton(
                      onPressed: _signinRoot,
                      child: const Text('Signin Root'),
                    ),
                    TextField(
                      controller: _tokenController,
                      decoration: const InputDecoration(
                        labelText: 'Token (JSON)',
                      ),
                    ),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                      children: [
                        ElevatedButton(
                          onPressed: _authenticate,
                          child: const Text('Authenticate'),
                        ),
                        ElevatedButton(
                          onPressed: _invalidate,
                          child: const Text('Invalidate'),
                        ),
                      ],
                    ),
                  ]),
                  _buildSection('Session', [
                    TextField(
                      controller: _nsController,
                      decoration: const InputDecoration(labelText: 'Namespace'),
                    ),
                    TextField(
                      controller: _dbController,
                      decoration: const InputDecoration(labelText: 'Database'),
                    ),
                    ElevatedButton(
                      onPressed: _useNsDb,
                      child: const Text('Use NS & DB'),
                    ),
                  ]),
                  _buildSection('CRUD & Query', [
                    ElevatedButton(
                      onPressed: _createPerson,
                      child: const Text('Create Random Person'),
                    ),
                    const SizedBox(height: 8),
                    TextField(
                      controller: _queryController,
                      decoration: const InputDecoration(
                        labelText: 'SurrealQL Query',
                      ),
                      maxLines: 3,
                    ),
                    ElevatedButton(
                      onPressed: _executeQuery,
                      child: const Text('Execute Query'),
                    ),
                  ]),
                  _buildSection('Transactions', [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                      children: [
                        ElevatedButton(
                          onPressed: _testTransactionCommit,
                          child: const Text('Test Commit'),
                        ),
                        ElevatedButton(
                          onPressed: _testTransactionCancel,
                          child: const Text('Test Cancel'),
                        ),
                      ],
                    ),
                  ]),
                ],
              ),
            ),
            const Divider(height: 1),
            const Padding(
              padding: EdgeInsets.all(4.0),
              child: Text(
                'Logs',
                style: TextStyle(fontWeight: FontWeight.bold),
              ),
            ),
            SizedBox(
              height: 200,
              child: ListView.builder(
                itemCount: _logs.length,
                itemBuilder: (context, index) {
                  return ListTile(
                    title: Text(
                      _logs[index],
                      style: const TextStyle(fontSize: 12),
                    ),
                    dense: true,
                    visualDensity: VisualDensity.compact,
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildSection(String title, List<Widget> children) {
    return Card(
      margin: const EdgeInsets.all(8.0),
      child: ExpansionTile(
        title: Text(title, style: const TextStyle(fontWeight: FontWeight.bold)),
        initiallyExpanded: true,
        children: [
          Padding(
            padding: const EdgeInsets.all(8.0),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: children,
            ),
          ),
        ],
      ),
    );
  }
}
