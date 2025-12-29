import 'package:flutter/material.dart';

class LiveQueryLogs extends StatelessWidget {
  final List<String> events;

  const LiveQueryLogs({super.key, required this.events});

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      itemCount: events.length,
      itemBuilder: (context, index) {
        final text = events[index];
        return Container(
          padding: const EdgeInsets.all(8),
          color: index % 2 == 0 ? Colors.black12 : Colors.transparent,
          child: SelectableText(
            text,
            style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
          ),
        );
      },
    );
  }
}
