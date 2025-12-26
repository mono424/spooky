import 'package:flutter/material.dart';
import 'controllers/spooky_controller.dart';
import 'components/status_bar.dart';
import 'components/logs_panel.dart';
import 'components/spooky_app_bar.dart';
import 'components/view_switcher.dart';

void main() {
  runApp(const MaterialApp(home: SpookyApp()));
}

class SpookyApp extends StatefulWidget {
  const SpookyApp({super.key});

  @override
  State<SpookyApp> createState() => _SpookyAppState();
}

class _SpookyAppState extends State<SpookyApp> with WidgetsBindingObserver {
  final SpookyController _controller = SpookyController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.detached) {
      _controller.client?.close();
    }
  }

  void _showErrorSnackBar(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(message), backgroundColor: Colors.red),
    );
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, child) {
        return Scaffold(
          appBar: SpookyAppBar(
            onDisconnect: _controller.disconnect,
            onLogout: _controller.logout,
            isLoggedIn: _controller.isLoggedIn,
          ),
          body: Column(
            children: [
              StatusBar(
                isInitialized: _controller.isInitialized,
                client: _controller.client,
              ),
              Expanded(
                child: Row(
                  children: [
                    Expanded(
                      flex: 3,
                      child: Padding(
                        padding: const EdgeInsets.all(24.0),
                        child: ViewSwitcher(
                          controller: _controller,
                          onError: _showErrorSnackBar,
                        ),
                      ),
                    ),
                    const VerticalDivider(width: 1),
                    Expanded(
                      flex: 2,
                      child: LogsPanel(
                        controller: _controller.logController,
                        scrollController: _controller.scrollController,
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
