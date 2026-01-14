import 'dart:io';

void main() async {
  final file = File('lib/schema/src/models.dart');
  if (!file.existsSync()) {
    print('File not found');
    exit(1);
  }

  String content = await file.readAsString();
  // Replace literal '\$' with '$', '\"' with '"', '\'' with "'"
  final newContent = content
      .replaceAll(r'\$', r'$')
      .replaceAll(r'\"', r'"')
      .replaceAll(r"\'", r"'");

  if (content != newContent) {
    await file.writeAsString(newContent);
    print('Fixed escapes in models.dart');
  } else {
    print('No escapes found to fix.');
  }
}
