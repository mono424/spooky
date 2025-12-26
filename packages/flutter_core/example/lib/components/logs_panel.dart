import 'package:flutter/material.dart';

class LogsPanel extends StatelessWidget {
  final TextEditingController controller;
  final ScrollController scrollController;

  const LogsPanel({
    super.key,
    required this.controller,
    required this.scrollController,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Container(
          padding: const EdgeInsets.all(8),
          color: Colors.grey.shade200,
          width: double.infinity,
          child: const Text(
            "Logs",
            style: TextStyle(fontWeight: FontWeight.bold),
          ),
        ),
        Expanded(
          child: Container(
            color: Colors.black87,
            child: TextField(
              controller: controller,
              scrollController: scrollController,
              readOnly: true,
              maxLines: null,
              style: const TextStyle(
                color: Colors.greenAccent,
                fontFamily: 'Courier',
                fontSize: 13,
              ),
              decoration: const InputDecoration(
                contentPadding: EdgeInsets.all(12),
                border: InputBorder.none,
              ),
            ),
          ),
        ),
      ],
    );
  }
}
