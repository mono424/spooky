import 'package:flutter/material.dart';
import '../../components/action_card.dart';

class DashboardView extends StatelessWidget {
  final VoidCallback onQueryRemote;
  final VoidCallback onSelectSchema;
  final VoidCallback onOpenLiveQuery;

  const DashboardView({
    super.key,
    required this.onQueryRemote,
    required this.onSelectSchema,
    required this.onOpenLiveQuery,
  });

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const Text(
          "Dashboard",
          style: TextStyle(fontWeight: FontWeight.bold, fontSize: 24),
        ),
        const SizedBox(height: 20),
        Wrap(
          spacing: 16,
          runSpacing: 16,
          children: [
            ActionCard(
              title: "Remote DB Info",
              icon: Icons.info_outline,
              onTap: onQueryRemote,
              color: Colors.blue.shade50,
            ),
            ActionCard(
              title: "Schema Query",
              icon: Icons.schema_outlined,
              onTap: onSelectSchema,
              color: Colors.orange.shade50,
            ),
            ActionCard(
              title: "Live Query",
              icon: Icons.broadcast_on_personal,
              onTap: onOpenLiveQuery,
              color: Colors.green.shade50,
            ),
          ],
        ),
      ],
    );
  }
}
