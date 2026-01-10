import 'package:flutter/material.dart';

class LiveQueryActions extends StatelessWidget {
  final VoidCallback onCreate;
  final VoidCallback onUpdate;

  const LiveQueryActions({
    super.key,
    required this.onCreate,
    required this.onUpdate,
  });

  @override
  Widget build(BuildContext context) {
    return Wrap(
      spacing: 16,
      children: [
        ElevatedButton.icon(
          onPressed: onCreate,
          icon: const Icon(Icons.add),
          label: const Text("Create Record"),
        ),
        ElevatedButton.icon(
          onPressed: onUpdate,
          icon: const Icon(Icons.update),
          label: const Text("Update All"),
        ),
      ],
    );
  }
}
