import 'package:flutter/material.dart';

class LiveQueryInput extends StatelessWidget {
  final TextEditingController controller;
  final bool isListening;
  final VoidCallback onToggle;

  const LiveQueryInput({
    super.key,
    required this.controller,
    required this.isListening,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(16.0),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: controller,
              decoration: const InputDecoration(labelText: "Table Name"),
            ),
          ),
          const SizedBox(width: 16),
          ElevatedButton(
            onPressed: onToggle,
            style: ElevatedButton.styleFrom(
              backgroundColor: isListening ? Colors.red : Colors.green,
            ),
            child: Text(isListening ? "Stop Listening" : "Start Listening"),
          ),
        ],
      ),
    );
  }
}
