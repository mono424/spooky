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
        title: const Text('Threads'),
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
              padding: const EdgeInsets.all(16),
              itemCount: _threads.length,
              itemBuilder: (context, index) {
                final thread = _threads[index];
                final title = thread['title'] ?? 'No Title';
                final content = thread['content'] ?? '';
                final id = thread['id'] ?? '';
                final createdAt = thread['created_at'] ?? '';
                
                // Author handling
                String authorId = 'Unknown';
                if (thread['author'] != null) {
                   authorId = thread['author'].toString(); 
                }
                
                final isMe = widget.controller.userId != null && 
                             authorId.contains(widget.controller.userId!);

                return Card(
                  margin: const EdgeInsets.only(bottom: 12),
                  // Visual distinction for "Me": slightly different color or border
                  color: isMe ? Theme.of(context).cardColor.withValues(alpha: 1.0) : Theme.of(context).cardColor.withValues(alpha: 0.8),
                  shape: isMe 
                      ? RoundedRectangleBorder(
                          side: BorderSide(color: Theme.of(context).primaryColor.withValues(alpha: 0.5), width: 1),
                          borderRadius: BorderRadius.circular(12))
                      : null,
                  child: InkWell(
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
                    child: Padding(
                      padding: const EdgeInsets.all(16.0),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              Expanded(
                                child: Text(
                                  title,
                                  style: GoogleFonts.outfit(
                                    fontWeight: FontWeight.bold,
                                    fontSize: 18,
                                  ),
                                ),
                              ),
                              if (isMe)
                                Container(
                                  margin: const EdgeInsets.only(right: 8),
                                  padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                                  decoration: BoxDecoration(
                                    color: Theme.of(context).primaryColor.withValues(alpha: 0.2),
                                    borderRadius: BorderRadius.circular(4),
                                  ),
                                  child: Text("YOU", style: const TextStyle(fontSize: 10, fontWeight: FontWeight.bold)),
                                ),
                              Text(
                                _formatDate(createdAt),
                                style: GoogleFonts.outfit(
                                  fontSize: 12,
                                  color: Colors.grey,
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 8),
                          Text(
                            content,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                            style: GoogleFonts.outfit(color: Colors.white70),
                          ),
                          const SizedBox(height: 12),
                          Row(
                            mainAxisAlignment: MainAxisAlignment.spaceBetween,
                            children: [
                              Text(
                                "by $authorId",
                                style: GoogleFonts.outfit(
                                  fontSize: 12,
                                  color: Colors.white38,
                                  fontStyle: FontStyle.italic
                                ),
                              ),
                              
                              if (isMe)
                                PopupMenuButton<String>(
                                  icon: const Icon(Icons.more_horiz, size: 20, color: Colors.grey),
                                  onSelected: (value) {
                                    if (value == 'edit') {
                                      _editThread(id, title, content);
                                    } else if (value == 'delete') {
                                      _deleteThread(id);
                                    }
                                  },
                                  itemBuilder: (context) => [
                                    const PopupMenuItem(
                                      value: 'edit',
                                      child: Text('Edit'),
                                    ),
                                    const PopupMenuItem(
                                      value: 'delete',
                                      child: Text('Delete', style: TextStyle(color: Colors.red)),
                                    ),
                                  ],
                                ),
                            ],
                          ),
                        ],
                      ),
                    ),
                  ),
                );
              },
            ),
    );
  }
  
  String _formatDate(String iso) {
    if (iso.isEmpty) return '';
    try {
      final dt = DateTime.parse(iso).toLocal();
      return "${dt.hour.toString().padLeft(2,'0')}:${dt.minute.toString().padLeft(2,'0')} ${dt.day}/${dt.month}";
    } catch (_) {
      return iso;
    }
  }

  Future<void> _deleteThread(String id) async {
    try {
      await widget.controller.delete(RecordId.fromString(id));
      widget.controller.log("Deleted thread: $id");
      // UI update via subscription usually, or manual refresh
      // _initQuery(); // Or wait for subscription
    } catch (e) {
      widget.controller.log("Delete Error: $e");
    }
  }

  void _editThread(String id, String currentTitle, String currentContent) {
    final titleController = TextEditingController(text: currentTitle);
    final contentController = TextEditingController(text: currentContent);

    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Edit Thread'),
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
              Navigator.pop(ctx);
              try {
                await widget.controller.update(RecordId.fromString(id), {
                  'title': titleController.text,
                  'content': contentController.text,
                });
                widget.controller.log("Updated thread: $id");
              } catch (e) {
                widget.controller.log("Update Error: $e");
              }
            },
            child: const Text('Save'),
          ),
        ],
      ),
    );
  }
}
