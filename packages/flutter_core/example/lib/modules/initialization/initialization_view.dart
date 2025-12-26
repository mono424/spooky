import 'package:flutter/material.dart';

class InitializationView extends StatelessWidget {
  final TextEditingController namespaceController;
  final TextEditingController databaseController;
  final TextEditingController endpointController;
  final TextEditingController devSidecarPortController;
  final bool useDevSidecar;
  final ValueChanged<bool?> onDevSidecarChanged;
  final VoidCallback onInit;

  const InitializationView({
    super.key,
    required this.namespaceController,
    required this.databaseController,
    required this.endpointController,
    required this.devSidecarPortController,
    required this.useDevSidecar,
    required this.onDevSidecarChanged,
    required this.onInit,
  });

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Text(
            "Initialize Spooky Client to Begin",
            style: TextStyle(fontSize: 18, fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 20),
          SizedBox(
            width: 300,
            child: TextField(
              controller: namespaceController,
              decoration: const InputDecoration(
                labelText: "Namespace",
                border: OutlineInputBorder(),
              ),
            ),
          ),
          const SizedBox(height: 10),
          SizedBox(
            width: 300,
            child: TextField(
              controller: databaseController,
              decoration: const InputDecoration(
                labelText: "Database",
                border: OutlineInputBorder(),
              ),
            ),
          ),
          const SizedBox(height: 10),
          SizedBox(
            width: 300,
            child: TextField(
              controller: endpointController,
              decoration: const InputDecoration(
                labelText: "Endpoint (Optional)",
                hintText: "ws://127.0.0.1:8000/rpc",
                border: OutlineInputBorder(),
              ),
            ),
          ),
          const SizedBox(height: 10),
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Checkbox(value: useDevSidecar, onChanged: onDevSidecarChanged),
              const Text("Enable Dev Sidecar (Host Local Server)"),
            ],
          ),
          if (useDevSidecar) ...[
            SizedBox(
              width: 300,
              child: TextField(
                controller: devSidecarPortController,
                decoration: const InputDecoration(
                  labelText: "Sidecar Port",
                  hintText: "5000",
                  border: OutlineInputBorder(),
                ),
                keyboardType: TextInputType.number,
              ),
            ),
            const SizedBox(height: 8),
            const Text(
              "Credentials: root / root",
              style: TextStyle(fontStyle: FontStyle.italic, color: Colors.grey),
            ),
          ],
          const SizedBox(height: 20),
          ElevatedButton.icon(
            onPressed: onInit,
            icon: const Icon(Icons.play_arrow),
            label: const Text("Initialize Client"),
            style: ElevatedButton.styleFrom(
              padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 16),
            ),
          ),
        ],
      ),
    );
  }
}
