pub mod predicate;
pub mod plan;
pub mod scan;
pub mod filter;
pub mod join;
pub mod semi_join;
pub mod anti_join;
pub mod union;
pub mod map;
pub mod top_k;
pub mod aggregate;
pub mod distinct;

use crate::algebra::ZSet;
use crate::circuit::store::Store;
use crate::types::Sp00kyValue;
use std::fmt::Debug;

/// A node in the DBSP circuit.
///
/// Each operator implements two evaluation modes derived from the
/// DBSP incrementalization theorem `Q_inc = D . lift(Q) . I`:
///
///   - `snapshot`: corresponds to `lift(Q)` — full evaluation from complete input Z-sets
///   - `step`: corresponds to the differentiated delta rule `D(Q)` — incremental
///     evaluation from input deltas, producing output deltas
///
/// Stateful operators (Join, TopK, Aggregate, Distinct) hold Z⁻¹
/// integration state internally and update it on each `step()` call.
/// Stateless operators (Scan, Filter, Map) have identical `snapshot` and `step`.
pub trait Operator: Debug + Send + Sync {
    /// Full evaluation: input Z-sets → output Z-set.
    ///
    /// Used for initial load and correctness verification.
    /// Does NOT modify internal state.
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, ctx: Option<&Sp00kyValue>) -> ZSet;

    /// Incremental evaluation: input deltas → output delta.
    ///
    /// Stateful operators update their Z⁻¹ integration state here.
    /// Always produces a delta (may be empty). Never returns None.
    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        ctx: Option<&Sp00kyValue>,
    ) -> ZSet;

    /// Number of input ports. Scan=0, unary operators=1, Join=2.
    fn arity(&self) -> usize;

    /// Reset all internal state (for re-initialization).
    fn reset(&mut self);

    /// Base collections this operator directly reads from (Scan only).
    fn collections(&self) -> Vec<String> {
        vec![]
    }

    /// Membership test for a single key against this operator's CURRENT
    /// snapshot output, without recomputing the full snapshot.
    ///
    /// Used by `Circuit::step_query` to detect membership transitions
    /// caused by `Operation::Update` (weight 0): the Scan emits an
    /// empty delta for Updates, so Filter never re-evaluates the
    /// predicate against the new row content. Walking the DAG with
    /// `evaluate_key` lets us ask "is this key in the current
    /// snapshot?" and synthesize +1/-1 weights when membership
    /// transitioned.
    ///
    /// `input_evals[i]` is the result of `evaluate_key` on the i-th
    /// input node, computed in topological order.
    ///
    /// Default: returns `false`. Conservative — operators that don't
    /// implement this won't surface membership transitions through
    /// Updates. Override on operators whose membership semantics are
    /// well-defined per-row (Scan, Filter, Map, TopK, Union, Distinct).
    fn evaluate_key(
        &self,
        _key: &str,
        _input_evals: &[bool],
        _store: &Store,
        _ctx: Option<&Sp00kyValue>,
    ) -> bool {
        false
    }
}

pub use aggregate::{Aggregate, AggregateFunc};
pub use distinct::Distinct;
pub use filter::Filter;
pub use join::Join;
pub use map::Map;
pub use plan::{JoinCondition, OperatorPlan, OrderSpec, Projection, QueryPlan};
pub use predicate::Predicate;
pub use anti_join::AntiJoin;
pub use scan::Scan;
pub use semi_join::SemiJoin;
pub use top_k::TopK;
pub use union::Union;
