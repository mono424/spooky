import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';
// import 'package:path_provider/path_provider.dart'; // Not needed for current dir
import 'schema/src/models.dart';

void main() {
  runApp(const MyApp());
}

class MyApp extends StatefulWidget {
  const MyApp({super.key});

  @override
  State<MyApp> createState() => _MyAppState();
}

class _MyAppState extends State<MyApp> {
  Spooky? _spooky;
  String _status = 'Initializing...';
  bool _initialized = false;
  String _dbPath = '';
  String _verificationInfo = '';

  @override
  void initState() {
    super.initState();
    _initSpooky();
  }

  Future<void> _initSpooky() async {
    try {
      // Use current directory (usually the project root when running via flutter)
      // final docsDir = Directory.current;
      // Hardcoded path as requested by user to target project source folder
      final dbPath =
          '/Users/timohty/projekts/spooky/packages/flutter_core/example/db';
      await Directory(dbPath).create(recursive: true);

      final localUrl = 'rocksdb://$dbPath';

      final config = SpookyConfig(
        schemaString: SURQL_SCHEMA,
        globalDBurl: '',
        localDBurl: localUrl,
        dbName: 'spooky',
        namespace: 'test',
        database: 'test',
        internalDatabase: 'spooky_internal',
      );

      final client = await Spooky.create(config);

      // Verify DB and Namespace
      // Note: queryLocal return type might need handling if it returns map/dynamic
      // Assuming toString() is sufficient for debug display
      final dbInfo = await client.db.queryLocal('INFO FOR DB');
      final nsInfo = await client.db.queryLocal('INFO FOR NS');

      if (!mounted) return;
      setState(() {
        _spooky = client;
        _status = 'Spooky Initialized!';
        _initialized = true;
        _dbPath = dbPath;
        _verificationInfo = 'DB Info: $dbInfo\nNS Info: $nsInfo';
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _status = 'Failed to initialize: $e';
        debugPrint('Failed to initialize: $e');
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      home: Scaffold(
        appBar: AppBar(title: const Text('Spooky Example')),
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(16.0),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text('Status: $_status', textAlign: TextAlign.center),
                if (_initialized) ...[
                  const SizedBox(height: 20),
                  const Text('Client is ready.'),
                  Text('Spooky hash: ${_spooky.hashCode}'),
                  const SizedBox(height: 10),
                  const Text(
                    'Database Path:',
                    style: TextStyle(fontWeight: FontWeight.bold),
                  ),
                  Text(_dbPath, textAlign: TextAlign.center),
                  const SizedBox(height: 10),
                  const Text(
                    'Verification:',
                    style: TextStyle(fontWeight: FontWeight.bold),
                  ),
                  Text(
                    _verificationInfo,
                    textAlign: TextAlign.center,
                    style: const TextStyle(fontSize: 12),
                  ),
                  // Add more example usage here if needed
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
