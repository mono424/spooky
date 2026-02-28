/// The weight type for Z-sets.
///
/// Weights are elements of the ring of integers (Z), which provides:
/// - Abelian group under addition: (Z, +, 0, -)
///   - Associative: (a + b) + c = a + (b + c)
///   - Commutative: a + b = b + a
///   - Identity: a + 0 = a
///   - Inverse: a + (-a) = 0
/// - Ring with multiplication: (Z, +, *, 0, 1)
///   - Used in join computation: weight_out = weight_left * weight_right
///
/// Positive weight (> 0) means the record is present.
/// Negative weight (< 0) means the record has been deleted.
/// Zero weight means the record is absent (and is removed from the map).
pub type Weight = i64;
