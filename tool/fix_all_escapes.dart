import 'dart:io';

Future<void> fixFile(String path) async {
  final file = File(path);
  if (!file.existsSync()) {
    print('File not found: $path');
    return;
  }

  String content = await file.readAsString();
  // Replace literal '\$' with '$', '\"' with '"', '\'' with "'"
  // Also fix escaped newlines if any? No, mainly variable escapes.
  final newContent = content
      .replaceAll(r'\$', r'$')
      .replaceAll(r'\"', r'"')
      .replaceAll(r"\'", r"'");

  if (content != newContent) {
    await file.writeAsString(newContent);
    print('Fixed escapes in $path');
  } else {
    print('No escapes found to fix in $path');
  }
}

void main() async {
  await fixFile('packages/flutter_core/example/lib/schema/src/models.dart');
  await fixFile('example/client-dart/src/db/models.dart');
}
