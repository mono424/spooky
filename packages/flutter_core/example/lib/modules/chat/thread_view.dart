import 'dart:async';
import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:flutter_core/flutter_core.dart';
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
          'SELECT * FROM comment WHERE thread = \$threadId ORDER BY created_at ASC';
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

      await widget.controller.create(id, {
        'content': text,
        'thread': widget.threadId,
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
                      final author = comment['author'] ?? 'Anon';

                      return Card(
                        margin: const EdgeInsets.only(bottom: 12),
                        child: Padding(
                          padding: const EdgeInsets.all(12),
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                content,
                                style: GoogleFonts.outfit(fontSize: 16),
                              ),
                              const SizedBox(height: 4),
                              Text(
                                "by $author",
                                style: GoogleFonts.outfit(
                                  fontSize: 12,
                                  color: Colors.grey,
                                ),
                              ),
                            ],
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
}
