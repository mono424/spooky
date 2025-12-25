import 'dart:convert';
import 'dart:io';

void main() async {
  print('Connecting to generic WebSocket ws://127.0.0.1:8000/rpc ...');

  try {
    final socket = await WebSocket.connect('ws://127.0.0.1:8000/rpc');
    print('Connected.');

    // Listen for messages
    socket.listen(
      (message) {
        print('Received: $message');

        final data = jsonDecode(message as String);
        if (data['id'] == 'signup_req') {
          if (data['error'] != null) {
            print('Signup Error: ${data['error']}');
            if (data['error'].toString().contains(
              'record access signup query failed',
            )) {
              print('FAILURE: Server is using OLD schema.');
              exit(1);
            } else {
              print('Other error occurred.');
              exit(1);
            }
          } else {
            print('Signup Success! Result: ${data['result']}');
            print('SUCCESS: Server is using NEW schema.');
            exit(0);
          }
        }
      },
      onError: (e) {
        print('WebSocket Error: $e');
        exit(1);
      },
      onDone: () {
        print('WebSocket Closed.');
      },
    );

    // Send Signup Request
    // Based on SurrealDB RPC format
    final username = 'raw_user_${DateTime.now().millisecondsSinceEpoch}';
    final request = {
      "id": "signup_req",
      "method": "signup",
      "params": [
        {
          "ns": "main",
          "db": "main",
          "access": "account",
          "username": username,
          "password": "password",
        },
      ],
    };

    print('Sending Signup: ${jsonEncode(request)}');
    socket.add(jsonEncode(request));

    // Wait for response or timeout
    await Future.delayed(Duration(seconds: 5));
    print('Timeout waiting for response.');
    exit(1);
  } catch (e) {
    print('Connection failed: $e');
    exit(1);
  }
}
