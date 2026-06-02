/// A single typed filter condition produced by a [TypedField] operator.
///
/// [op] is null for equality (the dynamic `QueryBuilder`'s plain-value branch);
/// otherwise it is a SurrealQL comparison operator mapped to a `QueryOp`.
class Condition {
  const Condition(this.field, this.op, this.value);
  final String field;
  final String? op; // null => equality
  final Object? value;
}

/// Base for generated, table-specific field tokens. Each subtype exposes only
/// the operators valid for its Dart type, so an invalid operand is a compile
/// error.
sealed class TypedField<T> {
  const TypedField(this.name);

  /// The declared (wire) field name.
  final String name;

  Condition eq(T value) => Condition(name, null, value);
  Condition ne(T value) => Condition(name, '!=', value);
}

/// Mixin adding ordered comparisons for numeric/temporal fields.
mixin _Comparable<T> on TypedField<T> {
  Condition gt(T value) => Condition(name, '>', value);
  Condition gte(T value) => Condition(name, '>=', value);
  Condition lt(T value) => Condition(name, '<', value);
  Condition lte(T value) => Condition(name, '<=', value);
}

class StringField extends TypedField<String> {
  const StringField(super.name);
}

class BoolField extends TypedField<bool> {
  const BoolField(super.name);
}

class NumField extends TypedField<num> with _Comparable<num> {
  const NumField(super.name);
}

class DateTimeField extends TypedField<DateTime> with _Comparable<DateTime> {
  const DateTimeField(super.name);
}

/// A record-reference field. Accepts a `RecordId` or its `table:id` string.
class RecordField extends TypedField<Object> {
  const RecordField(super.name);
}
