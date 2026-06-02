/// Typed-query runtime for generated spooky_core clients.
///
/// Generated code (via `spooky_gen`) builds field tokens, collections, and an
/// `AppDb` facade on top of these. Import alongside `spooky_core.dart`.
library;

export 'src/typed/fields.dart'
    show
        Condition,
        TypedField,
        StringField,
        BoolField,
        NumField,
        DateTimeField,
        RecordField;
export 'src/typed/typed_query.dart' show TypedQuery;
