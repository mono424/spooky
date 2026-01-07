import 'dart:async';
import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import '../../controllers/spooky_controller.dart';

class ThreadView extends StatefulWidget {
  final SpookyController controller;
  final String threadId;
  final String threadTitle;

  const ThreadView({
    super.key,
    required this.controller,
    required this.threadId,
    required this.threadTitle,
  });

  @override
  State<ThreadView> createState() => _ThreadViewState();
}

class _ThreadViewState extends State<ThreadView> {
  List<Map<String, dynamic>> _comments = [];
  bool _isLoading = true;
  Function()? _unsubscribe;
  final TextEditingController _commentController = TextEditingController();

  @override
  void initState() {
    super.initState();
  }

  // Custom hook-like init
  bool _initialized = false;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (!_initialized) {
      _initQuery();
      _initialized = true;
    }
  }

  @override
  void dispose() {
    _unsubscribe?.call();
    _commentController.dispose();
    super.dispose();
  }

  Future<void> _initQuery() async {
    if (widget.controller.client == null) return;

    try {
      // Parameterized query for safety and correct Live Query targeting
      final sql =
          "SELECT * FROM comment WHERE thread = \$threadId ORDER BY created_at ASC";
      final params = {'threadId': widget.threadId};

      // Register Query
      final hash = await widget.controller.client!.query(
        tableName: 'comment',
        surrealql: sql,
        params: params,
        ttl: QueryTimeToLive.oneHour,
      );

      // Subscribe
      final unsub = await widget.controller.client!.subscribe(hash, (records) {
        if (mounted) {
          setState(() {
            _comments = List.from(records);
            _comments.sort((a, b) {
              final da =
                  DateTime.tryParse(a['created_at'] ?? '') ?? DateTime(0);
              final db =
                  DateTime.tryParse(b['created_at'] ?? '') ?? DateTime(0);
              return da.compareTo(db); // Oldest first
            });
            _isLoading = false;
          });
        }
      }, immediate: true);
      _unsubscribe = unsub;
    } catch (e) {
      print("Init Query Error (ThreadView): $e");
      if (mounted) setState(() => _isLoading = false);
    }
  }

  Future<void> _postComment() async {
    final text = _commentController.text.trim();
    if (text.isEmpty) return;

    _commentController.clear();
    FocusScope.of(context).unfocus(); // Request focus drop

    try {
      final id = 'comment:${DateTime.now().millisecondsSinceEpoch}';

      await widget.controller.create(RecordId.fromString(id), {
        'content': text,
        'thread': widget.threadId,
        'author': RecordId.fromString(widget.controller.userId ?? 'user:rvkme6hk9ckgji6dlcvx'),
        // 'author': ... see ChatDashboard note.
      });
    } catch (e) {
      widget.controller.log("Error posting comment: $e");
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text("Failed to post: $e")));
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: Text(widget.threadTitle)),
      body: Column(
        children: [
          Expanded(
            child: _isLoading
                ? const Center(child: CircularProgressIndicator())
                : _comments.isEmpty
                ? const Center(child: Text("No comments yet. be the first!"))
                : ListView.builder(
                    padding: const EdgeInsets.all(16),
                    itemCount: _comments.length,
                    itemBuilder: (context, index) {
                      final comment = _comments[index];
                      final content = comment['content'] ?? '';
                      // Author handling
                      String author = 'Anon';
                      if (comment['author'] != null) {
                         author = comment['author'].toString(); 
                      }
                      
                      final createdAt = comment['created_at'] ?? '';
                      
                      final isMe = widget.controller.userId != null && 
                                   author.contains(widget.controller.userId!);

                      return Align(
                        alignment: isMe ? Alignment.centerRight : Alignment.centerLeft,
                        child: Container(
                          width: MediaQuery.of(context).size.width * 0.8,
                          child: Card(
                            margin: const EdgeInsets.only(bottom: 12),
                            color: isMe ? Theme.of(context).primaryColor.withValues(alpha: 0.1) : Theme.of(context).cardColor,
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.only(
                                topLeft: const Radius.circular(12),
                                topRight: const Radius.circular(12),
                                bottomLeft: isMe ? const Radius.circular(12) : Radius.zero,
                                bottomRight: isMe ? Radius.zero : const Radius.circular(12),
                              ),
                            ),
                            child: ListTile(
                              title: Text(
                                content,
                                style: GoogleFonts.outfit(fontSize: 16),
                              ),
                              subtitle: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  const SizedBox(height: 4),
                                  Row(
                                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                                    children: [
                                      Text(
                                        "by $author",
                                        style: GoogleFonts.outfit(
                                          fontSize: 12,
                                          color: Colors.grey,
                                          fontWeight: isMe ? FontWeight.bold : FontWeight.normal,
                                        ),
                                      ),
                                      Text(
                                        _formatDate(createdAt),
                                        style: GoogleFonts.outfit(fontSize: 10, color: Colors.grey),
                                      ),
                                    ],
                                  ),
                                ],
                              ),
                              trailing: isMe ? PopupMenuButton<String>(
                                icon: const Icon(Icons.more_vert, size: 16),
                                onSelected: (value) {
                                  if (value == 'edit') {
                                    _editComment(comment['id'], content);
                                  } else if (value == 'delete') {
                                    _deleteComment(comment['id']);
                                  }
                                },
                                itemBuilder: (context) => [
                                  const PopupMenuItem(
                                    value: 'edit',
                                    child: Text('Edit'),
                                    height: 32,
                                  ),
                                  const PopupMenuItem(
                                    value: 'delete',
                                    child: Text('Delete', style: TextStyle(color: Colors.red)),
                                    height: 32,
                                  ),
                                ],
                              ) : null,
                            ),
                          ),
                        ),
                      );
                    },
                  ),
          ),
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: Theme.of(context).cardColor,
              border: const Border(top: BorderSide(color: Colors.white10)),
            ),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _commentController,
                    decoration: const InputDecoration(
                      hintText: "Write a reply...",
                      border: InputBorder.none,
                    ),
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.send),
                  onPressed: _postComment,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _deleteComment(String id) async {
    try {
      await widget.controller.delete(RecordId.fromString(id));
      widget.controller.log("Deleted comment: $id");
    } catch (e) {
      widget.controller.log("Delete Error: $e");
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text("Failed to delete: $e")));
    }
  }

  void _editComment(String id, String currentContent) {
    final contentController = TextEditingController(text: currentContent);
    showDialog(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Edit Comment'),
        content: TextField(
          controller: contentController,
          decoration: const InputDecoration(labelText: 'Content'),
          maxLines: 3,
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
                  'content': contentController.text,
                });
                widget.controller.log("Updated comment: $id");
              } catch (e) {
                widget.controller.log("Update Error: $e");
                ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text("Failed to update: $e")));
              }
            },
            child: const Text('Save'),
          ),
        ],
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
}
