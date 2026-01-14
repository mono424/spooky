import 'package:flutter/material.dart';
import 'controllers/spooky_controller.dart';
import 'components/status_bar.dart';
import 'components/logs_panel.dart';
import 'components/spooky_app_bar.dart';
import 'components/view_switcher.dart';

import 'core/theme.dart';

void main() {
  runApp(const SpookyApp());
}

class SpookyApp extends StatelessWidget {
  const SpookyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: 'Spooky Example',
      theme: SpookyTheme.theme,
      home: const SpookyHome(),
    );
  }
}

class SpookyHome extends StatefulWidget {
  const SpookyHome({super.key});

  @override
  State<SpookyHome> createState() => _SpookyHomeState();
}

class _SpookyHomeState extends State<SpookyHome> with WidgetsBindingObserver {
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
                flex: 3,
                child: Padding(
                  padding: const EdgeInsets.all(24.0),
                  child: ViewSwitcher(
                    controller: _controller,
                    onError: _showErrorSnackBar,
                  ),
                ),
              ),
              const Divider(height: 1),
              Expanded(
                flex: 1,
                child: LogsPanel(
                  controller: _controller.logController,
                  scrollController: _controller.scrollController,
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
