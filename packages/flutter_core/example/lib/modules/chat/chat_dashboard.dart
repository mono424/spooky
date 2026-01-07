import 'dart:async';
import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import '../../controllers/spooky_controller.dart';
import 'thread_view.dart'; // Will create next

class ChatDashboard extends StatefulWidget {
  final SpookyController controller;

  const ChatDashboard({super.key, required this.controller});

  @override
  State<ChatDashboard> createState() => _ChatDashboardState();
}

class _ChatDashboardState extends State<ChatDashboard> {
  // Local state for threads
  List<Map<String, dynamic>> _threads = [];
  bool _isLoading = true;
  Function()? _unsubscribe;

  @override
  void initState() {
    super.initState();
    // Start query initialization
    _initQuery();
  }

  @override
  void dispose() {
    _unsubscribe?.call();
    super.dispose();
  }

  // Wrapper to register and subscribe
  Future<void> _initQuery() async {
    try {
      final sql = 'SELECT * FROM thread ORDER BY created_at DESC';
      final hash = await widget.controller.client!.query(
        tableName: 'thread',
        surrealql: sql,
        params: {},
        ttl: QueryTimeToLive.oneHour,
      );

      final unsub = await widget.controller.client!.subscribe(hash, (records) {
        if (mounted) {
          setState(() {
            _threads = List.from(records);
            _threads.sort((a, b) {
              final da =
                  DateTime.tryParse(a['created_at'] ?? '') ?? DateTime(0);
              final db =
                  DateTime.tryParse(b['created_at'] ?? '') ?? DateTime(0);
              return db.compareTo(da);
            });
            _isLoading = false;
          });
        }
      }, immediate: true);
      _unsubscribe = unsub;
    } catch (e) {
      print("Init Query Error: $e");
    }
  }

  void _createNewThread() {
    final titleController = TextEditingController();
    final contentController = TextEditingController();

    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('New Thread'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextField(
              controller: titleController,
              decoration: const InputDecoration(labelText: 'Title'),
            ),
            const SizedBox(height: 8),
            TextField(
              controller: contentController,
              decoration: const InputDecoration(labelText: 'Content'),
              maxLines: 3,
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          ElevatedButton(
            onPressed: () async {
              if (titleController.text.isEmpty) return;

              Navigator.pop(ctx);

              try {
                // Generate ID
                final id = 'thread:${DateTime.now().millisecondsSinceEpoch}';

                // Get current user (mocked or from auth state if we had it stored)
                // For now, let's assume we are authenticated.
                // The Schema expects 'author' field.
                // We'll trust the mutation manager or backend to validate.
                // But we need to pass data.

                if (widget.controller.userId == null) {
                  widget.controller.log(
                    "Error: User ID not found. Cannot create thread.",
                  );
                  return;
                }

                try {
                  // Use controller.create with explicit record field definition
                  // This handles:
                  // 1. Thread creation (Remote + Local)
                  // 2. Pending Mutation creation (Sync)
                  // 3. Record ID coercion (via recordFields)
                  await widget.controller.create(RecordId.fromString(id), {
                    'title': titleController.text,
                    'content': contentController.text,
                    'active': true,
                    'author': RecordId.fromString(widget.controller.userId ?? 'user:rvkme6hk9ckgji6dlcvx'),
                  });

                  widget.controller.log("Thread created");
                } catch (e) {
                  widget.controller.log("Creation Error: $e");
                  rethrow;
                }
                // if the Session isn't automatically applying it or we don't send it.
                // Schema: DEFINE FIELD author ... TYPE record<user>
                // PERMISSIONS ... create WHERE $access="account" AND author.id = $auth.id
                // So we MUST send author: $auth.id?
                // Usually SurrealDB $auth param handles it if we use `Create` but here we send data map.
                // But typically `author: $auth.id` could be a default value in schema?
                // Schema has no default.
                // So the USER must allow `author` to be set.
              } catch (e) {
                widget.controller.log("Error creating thread: $e");
              }
            },
            child: const Text('Post'),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    // _initQuery called in initState

    return Scaffold(
      appBar: AppBar(
        title: const Text('Spooky Chat - Threads'),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: _isLoading
                ? null
                : () {
                    _unsubscribe?.call();
                    _unsubscribe = null;
                    setState(() => _isLoading = true);
                    _initQuery();
                  },
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: _createNewThread,
        child: const Icon(Icons.add),
      ),
      body: _isLoading
          ? const Center(child: CircularProgressIndicator())
          : ListView.builder(
              itemCount: _threads.length,
              itemBuilder: (context, index) {
                final thread = _threads[index];
                final title = thread['title'] ?? 'No Title';
                final content = thread['content'] ?? '';
                final id = thread['id'] ?? '';

                return ListTile(
                  title: Text(
                    title,
                    style: GoogleFonts.outfit(fontWeight: FontWeight.bold),
                  ),
                  subtitle: Text(
                    content,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                  onTap: () {
                    Navigator.push(
                      context,
                      MaterialPageRoute(
                        builder: (_) => ThreadView(
                          controller: widget.controller,
                          threadId: id,
                          threadTitle: title,
                        ),
                      ),
                    );
                  },
                );
              },
            ),
    );
  }
}
