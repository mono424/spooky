import 'package:flutter/material.dart';
import 'package:flutter_core_example/core/theme.dart';
import 'package:google_fonts/google_fonts.dart';

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
      child: SingleChildScrollView(
        // Make it responsive
        padding: const EdgeInsets.all(24),
        child: Container(
          constraints: const BoxConstraints(maxWidth: 480), // Slightly wider
          padding: const EdgeInsets.all(40), // More internal padding
          decoration: BoxDecoration(
            color: SpookyColors.surface,
            borderRadius: BorderRadius.circular(4),
            border: Border.all(color: SpookyColors.white10),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                "Initialize Spooky Client ",

                style: GoogleFonts.outfit(
                  fontSize: 28,
                  fontWeight: FontWeight.bold,
                  color: SpookyColors.white,
                ),
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 48), // Increased spacing
              TextField(
                controller: namespaceController,
                decoration: const InputDecoration(
                  labelText: "Namespace",
                  prefixIcon: Icon(Icons.folder_open_outlined),
                ),
                style: const TextStyle(color: SpookyColors.white),
              ),
              const SizedBox(height: 24), // Increased spacing
              TextField(
                controller: databaseController,
                decoration: const InputDecoration(
                  labelText: "Database",
                  prefixIcon: Icon(Icons.storage_outlined),
                ),
                style: const TextStyle(color: SpookyColors.white),
              ),
              const SizedBox(height: 24), // Increased spacing
              TextField(
                controller: endpointController,
                decoration: const InputDecoration(
                  labelText: "Endpoint (Optional)",
                  hintText: "ws://127.0.0.1:8000/rpc",
                  prefixIcon: Icon(Icons.cloud_outlined),
                ),
                style: const TextStyle(color: SpookyColors.white),
              ),
              const SizedBox(height: 24),
              Container(
                decoration: BoxDecoration(
                  color: SpookyColors.background,
                  border: Border.all(color: SpookyColors.white),
                ),
                child: CheckboxListTile(
                  value: useDevSidecar,
                  onChanged: onDevSidecarChanged,
                  title: const Text("Enable Dev"),
                  activeColor: SpookyColors.white,
                  checkColor: SpookyColors.background,
                  contentPadding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 2,
                  ),
                ),
              ),
              if (useDevSidecar) ...[
                const SizedBox(height: 24), // Increased spacing
                TextField(
                  controller: devSidecarPortController,
                  decoration: const InputDecoration(
                    labelText: "Sidecar Port",
                    hintText: "5000",
                    prefixIcon: Icon(Icons.lan_outlined),
                  ),
                  keyboardType: TextInputType.number,
                  style: const TextStyle(color: SpookyColors.white),
                ),
                const SizedBox(height: 12),
                Text(
                  "Credentials: root / root",
                  style: GoogleFonts.inter(
                    fontStyle: FontStyle.italic,
                    color: SpookyColors.white60,
                    fontSize: 12,
                  ),
                  textAlign: TextAlign.center,
                ),
              ],
              const SizedBox(height: 24),
              SizedBox(
                height: 56,
                child: ElevatedButton.icon(
                  onPressed: onInit,
                  icon: const Icon(Icons.bolt),
                  label: const Text("Initialize Client"),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
