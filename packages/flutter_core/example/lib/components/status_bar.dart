import 'package:flutter/material.dart';
import 'package:flutter_core/flutter_core.dart';

class StatusBar extends StatelessWidget {
  final bool isInitialized;
  final SpookyClient? client;

  const StatusBar({super.key, required this.isInitialized, this.client});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(8),
      color: isInitialized ? Colors.green.shade100 : Colors.red.shade100,
      width: double.infinity,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(
            client != null ? Icons.check_circle : Icons.error,
            size: 16,
            color: isInitialized ? Colors.green.shade800 : Colors.red.shade800,
          ),
          const SizedBox(width: 8),
          Text(
            isInitialized ? "Client Initialized" : "Client Not Initialized",
            style: TextStyle(
              fontWeight: FontWeight.bold,
              color: isInitialized
                  ? Colors.green.shade800
                  : Colors.red.shade800,
            ),
          ),
        ],
      ),
    );
  }
}
