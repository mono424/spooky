// 1. Die Datenmodelle (was in den Zeilen steht)
class User {
  final int id;
  final String name;
  User(this.id, this.name);
}

class Product {
  final String sku;
  final double price;
  Product(this.sku, this.price);
}

// 2. Die Tabellen-Definition
// T ist der "Row-Type" (User oder Product)
abstract class Table<T> {
  String get name;
  List<T> get data;
}

class UsersTable extends Table<User> {
  @override
  String get name => "users";

  @override
  List<User> get data => [User(1, "Alice"), User(2, "Bob")];
}

class ProductsTable extends Table<Product> {
  @override
  String get name => "products";

  @override
  List<Product> get data => [Product("A100", 9.99)];
}

// 3. Die Schema-Klasse
class MySchema {
  // Wir nutzen hier Instanzen statt Typ-Magie
  final users = UsersTable();
  final products = ProductsTable();

  // Eine generische Methode, die den Typ T beibehält
  T getTable<T extends Table>(T table) {
    return table;
  }
}

void main() {
  var schema = MySchema();

  // Die IDE weiß hier sofort: 'userTable' ist vom Typ 'UsersTable'
  var userTable = schema.getTable(schema.users);

  // Die Autovervollständigung zeigt hier .name und .id an
  print(userTable.data.first.name);

  var prodTable = schema.getTable(schema.products);
  // Hier zeigt die IDE .sku und .price an
  print(prodTable.data.first.sku);
}
