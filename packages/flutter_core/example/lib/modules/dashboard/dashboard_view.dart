import 'package:flutter/material.dart';
import '../../components/action_card.dart';

class DashboardView extends StatelessWidget {
  final VoidCallback onQueryRemote;
  final VoidCallback onSelectSchema;
  final VoidCallback onOpenLiveQuery;
  final VoidCallback onOpenChat;
  final VoidCallback onOpenUpsyncTest;

  const DashboardView({
    super.key,
    required this.onQueryRemote,
    required this.onSelectSchema,
    required this.onOpenLiveQuery,
    required this.onOpenChat,
    required this.onOpenUpsyncTest,
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
            ),
            ActionCard(
              title: "Schema Query",
              icon: Icons.schema_outlined,
              onTap: onSelectSchema,
            ),
            ActionCard(
              title: "Live Query",
              icon: Icons.broadcast_on_personal,
              onTap: onOpenLiveQuery,
            ),
            ActionCard(
              title: "Spooky Chat",
              icon: Icons.chat_bubble_outline,
              onTap: onOpenChat,
            ),
            ActionCard(
              title: "Upsync Test",
              icon: Icons.sync,
              onTap: onOpenUpsyncTest,
            ),
          ],
        ),
      ],
    );
  }
}
