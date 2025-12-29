import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import '../../controllers/spooky_controller.dart';
import '../../core/theme.dart';

class LiveQueryView extends StatefulWidget {
  final SpookyController controller;

  const LiveQueryView({super.key, required this.controller});

  @override
  State<LiveQueryView> createState() => _LiveQueryViewState();
}

class _LiveQueryViewState extends State<LiveQueryView> {
  Stream<List<Map<String, dynamic>>>? _stream;

  @override
  void initState() {
    super.initState();
    _startStream();
  }

  void _startStream() {
    final client = widget.controller.client;
    // Ensure client is ready and connected
    if (client != null && client.local != null) {
      if (mounted) {
        setState(() {
          _stream = client.local!.getClient
              .select(resource: 'user')
              .live<Map<String, dynamic>>((json) => json);
        });
      }
    }
  }

  Future<void> _createUser() async {
    final client = widget.controller.client;
    if (client == null) {
      _showSnack("Client is null/not initialized!", isError: true);
      return;
    }

    final id = Random().nextInt(10000);
    final data = {
      "username": "user_$id",
      "status": "active",
      "created_at": DateTime.now().toIso8601String(),
    };

    try {
      await client.local!.getClient.create(
        resource: 'user',
        data: jsonEncode(data),
      );
      _showSnack("Created user_$id");
    } catch (e) {
      _showSnack("Create failed: $e", isError: true);
    }
  }

  Future<void> _deleteUser(String id) async {
    final client = widget.controller.client;
    if (client == null) return;
    try {
      await client.local!.getClient.delete(resource: id);
      _showSnack("Deleted $id");
    } catch (e) {
      _showSnack("Delete failed: $e", isError: true);
    }
  }

  void _showSnack(String msg, {bool isError = false}) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(msg, style: const TextStyle(color: Colors.white)),
        backgroundColor: isError ? Colors.red : SpookyColors.primary,
        duration: const Duration(milliseconds: 800),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: SpookyColors.background,
      appBar: AppBar(
        title: Text(
          "VISUAL DEMO",
          style: GoogleFonts.spaceMono(
            fontWeight: FontWeight.bold,
            letterSpacing: 1.2,
            color: SpookyColors.white,
          ),
        ),
        backgroundColor: SpookyColors.background,
        iconTheme: const IconThemeData(color: SpookyColors.white),
        elevation: 0,
        centerTitle: true,
      ),
      body: Column(
        children: [
          Container(
            padding: const EdgeInsets.all(16),
            width: double.infinity,
            decoration: const BoxDecoration(
              color: SpookyColors.card,
              border: Border(bottom: BorderSide(color: SpookyColors.white10)),
            ),
            child: Text(
              "StreamBuilder<List<User>>",
              style: GoogleFonts.firaCode(
                color: SpookyColors.secondary,
                fontWeight: FontWeight.bold,
              ),
              textAlign: TextAlign.center,
            ),
          ),
          Expanded(
            child: StreamBuilder<List<Map<String, dynamic>>>(
              stream: _stream,
              builder: (context, snapshot) {
                if (snapshot.hasError) {
                  return Center(
                    child: Padding(
                      padding: const EdgeInsets.all(16.0),
                      child: Text(
                        "Error: ${snapshot.error}",
                        style: const TextStyle(color: Colors.red),
                      ),
                    ),
                  );
                }
                if (!snapshot.hasData) {
                  return const Center(
                    child: CircularProgressIndicator(
                      color: SpookyColors.primary,
                    ),
                  );
                }

                final users = snapshot.data!;
                if (users.isEmpty) {
                  return Center(
                    child: Text(
                      "No users found.\nTap + to create one.",
                      textAlign: TextAlign.center,
                      style: GoogleFonts.spaceMono(color: SpookyColors.white60),
                    ),
                  );
                }

                return ListView.builder(
                  itemCount: users.length,
                  itemBuilder: (context, index) {
                    final user = users[index];
                    final id = user['id'] ?? 'unknown';
                    return Container(
                      margin: const EdgeInsets.symmetric(
                        horizontal: 16,
                        vertical: 8,
                      ),
                      decoration: BoxDecoration(
                        color: SpookyColors.card,
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(color: SpookyColors.white10),
                      ),
                      child: ListTile(
                        leading: CircleAvatar(
                          backgroundColor: SpookyColors.primary.withOpacity(
                            0.2,
                          ),
                          child: Text(
                            (user['username']?[0] ?? '?').toUpperCase(),
                            style: const TextStyle(color: SpookyColors.primary),
                          ),
                        ),
                        title: Text(
                          user['username'] ?? 'No Name',
                          style: const TextStyle(
                            color: SpookyColors.white,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                        subtitle: Text(
                          "ID: $id\nStatus: ${user['status'] ?? 'N/A'}",
                          style: const TextStyle(
                            color: SpookyColors.white60,
                            fontSize: 12,
                          ),
                        ),
                        isThreeLine: true,
                        trailing: IconButton(
                          icon: const Icon(
                            Icons.delete_outline,
                            color: Colors.redAccent,
                          ),
                          onPressed: () => _deleteUser(id),
                        ),
                      ),
                    );
                  },
                );
              },
            ),
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: _createUser,
        backgroundColor: SpookyColors.primary,
        child: const Icon(Icons.add, color: Colors.black),
      ),
    );
  }
}
