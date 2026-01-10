import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import '../../../../core/theme.dart';

class LiveQueryLogs extends StatelessWidget {
  final List<LogEntry> logs;
  final ScrollController scrollController;
  final VoidCallback onClear;

  const LiveQueryLogs({
    super.key,
    required this.logs,
    required this.scrollController,
    required this.onClear,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // Header (Matches LogsPanel)
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: const BoxDecoration(
            color: Colors.black, // Strict Black
            border: Border(bottom: BorderSide(color: Colors.white, width: 1)),
          ),
          width: double.infinity,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Row(
                children: [
                  const Icon(
                    Icons.terminal,
                    color: SpookyColors.white,
                    size: 14,
                  ),
                  const SizedBox(width: 8),
                  Text(
                    "LIVE LOGS",
                    style: GoogleFonts.spaceMono(
                      fontWeight: FontWeight.bold,
                      color: SpookyColors.white,
                      fontSize: 11,
                      letterSpacing: 0.5,
                    ),
                  ),
                ],
              ),
              IconButton(
                icon: const Icon(
                  Icons.delete_sweep,
                  color: SpookyColors.white60,
                  size: 16,
                ),
                onPressed: onClear,
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints(),
                tooltip: "Clear Logs",
              ),
            ],
          ),
        ),
        // Content
        Expanded(
          child: Container(
            color: Colors.black, // Pure black
            child: ListView.separated(
              controller: scrollController,
              padding: const EdgeInsets.all(12),
              itemCount: logs.length,
              separatorBuilder: (_, __) => Divider(
                color: SpookyColors.white.withOpacity(0.05),
                height: 1,
              ),
              itemBuilder: (context, index) {
                final log = logs[index];
                return _LogItem(log: log);
              },
            ),
          ),
        ),
      ],
    );
  }
}

class LogEntry {
  final String message;
  final DateTime timestamp;
  final bool isError;
  final bool isLive;

  LogEntry({
    required this.message,
    required this.timestamp,
    this.isError = false,
    this.isLive = false,
  });
}

class _LogItem extends StatelessWidget {
  final LogEntry log;

  const _LogItem({required this.log});

  @override
  Widget build(BuildContext context) {
    Color textColor = SpookyColors.white.withOpacity(0.7);
    if (log.isError) textColor = Colors.redAccent;
    if (log.isLive) textColor = Colors.greenAccent;

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            "${log.timestamp.hour.toString().padLeft(2, '0')}:${log.timestamp.minute.toString().padLeft(2, '0')}:${log.timestamp.second.toString().padLeft(2, '0')}",
            style: GoogleFonts.spaceMono(
              color: SpookyColors.white.withOpacity(0.3),
              fontSize: 10,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: SelectableText(
              log.message,
              style: GoogleFonts.spaceMono(color: textColor, fontSize: 12),
            ),
          ),
        ],
      ),
    );
  }
}
