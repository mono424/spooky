# DBSP Module Testing Strategy

A bottom-up testing approach for complete understanding and coverage of your DBSP stream processing module.

---

## Testing Philosophy

**Bottom-up approach**: Start with the foundational types and pure functions that have no dependencies, then progressively test components that build on top of them. This ensures:

1. You understand each piece before combining them
2. Test failures pinpoint the exact issue
3. Complex integration tests have solid foundations

---

## Phase 1: Foundational Types (No Dependencies)

These are pure data structures and functions with no external dependencies. Test these first to build your foundation.

### 1.1 Path (`types/path.rs` equivalent)

**Purpose**: Dot-notation field path traversal (e.g., `"user.profile.name"`)

**Unit Tests**:
```
□ test_path_new_empty - Path::new("") produces empty vec
□ test_path_new_single - Path::new("id") produces ["id"]
□ test_path_new_nested - Path::new("a.b.c") produces ["a", "b", "c"]
□ test_path_as_str - Roundtrip: Path::new("a.b") -> as_str() == "a.b"
□ test_path_is_empty - Empty path returns true
□ test_path_segments - segments() returns correct slice
□ test_path_serialize_deserialize - JSON roundtrip preserves value
```

**Why first**: Path is used everywhere (predicates, projections, field lookups). Must be rock solid.

---

### 1.2 SpookyValue (`types/spooky_value.rs` equivalent)

**Purpose**: Optimized JSON-like data structure for internal processing

**Unit Tests**:
```
□ test_spooky_null - SpookyValue::Null works
□ test_spooky_bool - true/false conversions
□ test_spooky_number - f64 precision, special values (NaN, Infinity)
□ test_spooky_string - SmolStr usage, as_str()
□ test_spooky_array - Vec operations
□ test_spooky_object - FastMap operations, key lookup
□ test_spooky_from_json_null
□ test_spooky_from_json_primitives - bool, number, string
□ test_spooky_from_json_array
□ test_spooky_from_json_nested_object
□ test_spooky_to_json_roundtrip - SpookyValue -> Value -> SpookyValue
□ test_spooky_get_nested - obj.get("key") works
□ test_spooky_is_null
□ test_spooky_as_object_returns_none_for_non_object
□ test_spooky_default - Default is Null
```

**Why second**: Used by every other component for data representation.

---

### 1.3 ZSet Types & Operations (`types/zset.rs` equivalent)

**Purpose**: Weighted multisets for incremental computation

**Unit Tests - Basic Types**:
```
□ test_make_zset_key_simple - "user" + "123" => "user:123"
□ test_make_zset_key_strips_prefix - "user" + "user:123" => "user:123" (no double prefix)
□ test_make_zset_key_inline_optimization - Keys ≤23 chars use SmolStr inline storage
□ test_parse_zset_key_valid - "user:123" => Some(("user", "123"))
□ test_parse_zset_key_no_colon - "invalid" => None
```

**Unit Tests - ZSetOps (Full DBSP Semantics)**:
```
□ test_zset_add_delta_simple - {a:1} + {a:1} = {a:2}
□ test_zset_add_delta_removal - {a:1} + {a:-1} = {} (removes zero weights)
□ test_zset_diff_additions - old:{a:1} vs new:{a:1,b:1} => {b:1}
□ test_zset_diff_removals - old:{a:1,b:1} vs new:{a:1} => {b:-1}
□ test_zset_diff_mixed - old:{a:1,b:1} vs new:{b:1,c:1} => {a:-1,c:1}
□ test_zset_diff_multiplicity - old:{a:1} vs new:{a:3} => {a:2}
□ test_weight_transition_inserted - 0->1 = Inserted
□ test_weight_transition_deleted - 1->0 = Deleted
□ test_weight_transition_unchanged - 1->1 = Unchanged
□ test_weight_transition_multiplicity_increased - 1->2
□ test_weight_transition_multiplicity_decreased - 2->1
□ test_membership_changes - Only returns Inserted/Deleted, not multiplicity changes
```

**Unit Tests - ZSetMembershipOps (Simplified for Edge System)**:
```
□ test_membership_is_member_positive_weight - weight > 0 = member
□ test_membership_is_member_zero_weight - weight = 0 = NOT member
□ test_membership_is_member_negative_weight - weight < 0 = NOT member
□ test_membership_is_member_not_in_map - missing key = NOT member
□ test_membership_add_member_idempotent - Adding twice keeps weight=1
□ test_membership_remove_member - Removes key from map
□ test_membership_apply_delta_normalizes - Any positive -> 1, any <=0 -> removed
□ test_membership_diff - Returns (additions, removals) ignoring weight changes
□ test_membership_diff_ignores_multiplicity - {a:1} vs {a:5} => no change
□ test_membership_diff_into - Populates provided ZSet (avoids alloc)
□ test_normalize_to_membership - {a:5, b:0, c:-1} => {a:1}
□ test_member_count - Counts only weight > 0
```

**Why third**: ZSet algebra is the mathematical foundation of DBSP. Every view operation uses these.

---

### 1.4 Operation & Delta Types (`types/circuit_types.rs` equivalent)

**Purpose**: Represent mutations and their effects

**Unit Tests**:
```
□ test_operation_from_str_create - "create" -> Create
□ test_operation_from_str_update - "update" -> Update
□ test_operation_from_str_delete - "delete" -> Delete
□ test_operation_from_str_invalid - "foo" -> None
□ test_operation_weight_create - Create = +1
□ test_operation_weight_update - Update = 0
□ test_operation_weight_delete - Delete = -1
□ test_operation_changes_content - Create/Update = true, Delete = false
□ test_operation_changes_membership - Create/Delete = true, Update = false
□ test_delta_new - Constructs correctly
□ test_delta_from_operation - Sets all fields correctly
□ test_delta_content_update - weight=0, content_changed=true
```

---

### 1.5 BatchDeltas (`types/batch_deltas.rs` equivalent)

**Purpose**: Group deltas by table for batch processing

**Unit Tests**:
```
□ test_batch_deltas_new_empty
□ test_batch_deltas_add_create - Adds to membership (+1) and content_updates
□ test_batch_deltas_add_update - Adds to content_updates only (weight=0)
□ test_batch_deltas_add_delete - Adds to membership (-1) only
□ test_batch_deltas_is_empty - True when both maps empty
□ test_batch_deltas_changed_tables - Returns union of both map keys
□ test_batch_deltas_multiple_ops_same_table - Aggregates correctly
```

---

## Phase 2: Evaluation Helpers (Pure Functions)

These are stateless functions that transform data. They depend on Phase 1 types.

### 2.1 resolve_nested_value

**Purpose**: Traverse SpookyValue using Path

**Unit Tests**:
```
□ test_resolve_empty_path - Returns root unchanged
□ test_resolve_single_level - {"a": 1} with "a" => 1
□ test_resolve_nested - {"a": {"b": 1}} with "a.b" => 1
□ test_resolve_missing_key - {"a": 1} with "b" => None
□ test_resolve_non_object_intermediate - {"a": 1} with "a.b" => None (a is not object)
□ test_resolve_null_root - None input => None
□ test_resolve_deep_nesting - 5+ levels deep
```

---

### 2.2 compare_spooky_values

**Purpose**: Comparison for sorting and predicates

**Unit Tests**:
```
□ test_compare_nulls - Null == Null
□ test_compare_bools - false < true
□ test_compare_numbers - Numeric ordering, NaN handling
□ test_compare_strings - Lexicographic
□ test_compare_arrays_by_length - Shorter < Longer
□ test_compare_arrays_same_length - Element-wise comparison
□ test_compare_objects_by_length - Compare by key count
□ test_compare_different_types - Uses type_rank ordering
□ test_compare_none_vs_some - None < Some
□ test_compare_both_none - Equal
```

---

### 2.3 hash_spooky_value

**Purpose**: Fast hashing for join keys

**Unit Tests**:
```
□ test_hash_null_deterministic - Same hash every time
□ test_hash_primitives_differ - Different types produce different hashes
□ test_hash_equal_values_equal_hash - {a:1} == {a:1} => same hash
□ test_hash_different_values_different_hash - {a:1} != {a:2} => different hash
□ test_hash_nested_structures - Nested objects hash correctly
```

---

### 2.4 normalize_record_id

**Purpose**: Convert RecordId objects to "table:id" string format

**Unit Tests**:
```
□ test_normalize_record_id_object_tb_id - {"tb":"user","id":"123"} => "user:123"
□ test_normalize_record_id_object_table_id - {"table":"user","id":"123"} => "user:123"
□ test_normalize_record_id_already_string - "user:123" => unchanged
□ test_normalize_record_id_non_record_object - {other:fields} => unchanged
□ test_normalize_record_id_numeric_parts - {"tb":1,"id":2} => "1:2"
```

---

### 2.5 NumericFilterConfig & SIMD Filtering

**Purpose**: Optimized numeric predicate evaluation

**Unit Tests**:
```
□ test_numeric_filter_config_from_gt - Extracts path, target, NumericOp::Gt
□ test_numeric_filter_config_from_gte
□ test_numeric_filter_config_from_lt
□ test_numeric_filter_config_from_lte
□ test_numeric_filter_config_from_eq
□ test_numeric_filter_config_from_neq
□ test_numeric_filter_config_non_numeric - Returns None for string predicates
□ test_filter_f64_batch_gt - Filters values > target
□ test_filter_f64_batch_lte - Filters values <= target
□ test_filter_f64_batch_eq_epsilon - Handles floating point equality
□ test_filter_f64_batch_empty_input - Returns empty
□ test_filter_f64_batch_large_input - Handles >8 elements (SIMD chunks)
□ test_extract_number_column - Extracts (keys, weights, numbers) from ZSet
```

---

## Phase 3: Operators & Predicates

### 3.1 Predicate (enum and check_predicate function)

**Purpose**: SQL WHERE clause conditions

**Unit Tests**:
```
□ test_predicate_eq_string_match
□ test_predicate_eq_string_no_match
□ test_predicate_eq_number
□ test_predicate_neq
□ test_predicate_gt_number
□ test_predicate_gte_number
□ test_predicate_lt_number
□ test_predicate_lte_number
□ test_predicate_prefix_match - "user:*" matches "user:123"
□ test_predicate_prefix_no_match
□ test_predicate_prefix_on_id_field - Special case optimization
□ test_predicate_and_all_true
□ test_predicate_and_one_false - Short-circuits
□ test_predicate_or_one_true - Short-circuits
□ test_predicate_or_all_false
□ test_predicate_nested_and_or - (A AND B) OR (C AND D)
□ test_predicate_with_param_context - $parent.author resolution
□ test_predicate_missing_field - Returns false
```

---

### 3.2 Operator (enum)

**Purpose**: Query plan tree nodes

**Unit Tests**:
```
□ test_operator_scan_serialization - JSON roundtrip
□ test_operator_filter_serialization
□ test_operator_join_serialization
□ test_operator_project_serialization
□ test_operator_limit_serialization
□ test_operator_referenced_tables_scan - ["user"]
□ test_operator_referenced_tables_filter - Passes through from input
□ test_operator_referenced_tables_join - Combines left + right
□ test_operator_referenced_tables_project_with_subquery - Includes subquery tables
□ test_operator_referenced_tables_deduplicates - Same table twice => once
□ test_operator_has_subquery_projections_true
□ test_operator_has_subquery_projections_false
□ test_operator_nested_subqueries
```

---

### 3.3 Projection & OrderSpec & JoinCondition

**Unit Tests**:
```
□ test_projection_all_serialization
□ test_projection_field_serialization
□ test_projection_subquery_serialization
□ test_order_spec_asc_default
□ test_order_spec_desc
□ test_join_condition_serialization
```

---

## Phase 4: Converter (SQL Parser)

### 4.1 SQL Parsing Helpers

**Unit Tests**:
```
□ test_parse_identifier_simple - "user" parses
□ test_parse_identifier_with_underscore - "user_id"
□ test_parse_identifier_with_colon - "user:123"
□ test_parse_identifier_with_dots - "user.profile"
□ test_parse_string_literal_single_quotes - 'hello'
□ test_parse_string_literal_double_quotes - "hello"
□ test_parse_string_literal_prefix - 'user:*' => Prefix
□ test_parse_value_entry_number - 42
□ test_parse_value_entry_bool_true
□ test_parse_value_entry_bool_false
□ test_parse_value_entry_param - $parent.id
```

---

### 4.2 Predicate Parsing

**Unit Tests**:
```
□ test_parse_leaf_predicate_eq - "status = 'active'"
□ test_parse_leaf_predicate_neq - "status != 'deleted'"
□ test_parse_leaf_predicate_gt - "age > 18"
□ test_parse_leaf_predicate_gte - "age >= 21"
□ test_parse_leaf_predicate_lt - "price < 100"
□ test_parse_leaf_predicate_lte - "count <= 10"
□ test_parse_and_expression - "a = 1 AND b = 2"
□ test_parse_or_expression - "a = 1 OR b = 2"
□ test_parse_nested_logic - "(a = 1 AND b = 2) OR c = 3"
□ test_parse_join_candidate - "id = other_table.id" => __JOIN_CANDIDATE__
```

---

### 4.3 Full Query Parsing (convert_surql_to_dbsp)

**Unit Tests**:
```
□ test_simple_select_star - "SELECT * FROM user"
□ test_select_with_where - "SELECT * FROM user WHERE status = 'active'"
□ test_select_with_limit - "SELECT * FROM user LIMIT 10"
□ test_select_with_order_by_asc - "SELECT * FROM user ORDER BY name ASC"
□ test_select_with_order_by_desc - "SELECT * FROM user ORDER BY created_at DESC"
□ test_select_with_order_limit - Combined ORDER BY + LIMIT
□ test_select_with_subquery - "(SELECT * FROM author WHERE id = $parent.author_id)[0] AS author"
□ test_select_multiple_fields - "SELECT id, name, email FROM user"
□ test_select_complex_where - Multiple AND/OR conditions
□ test_select_with_join - Implicit join via identifier comparison
□ test_sql_with_trailing_semicolon - Handles optional semicolon
□ test_sql_with_mixed_case - "SELECT", "select", "SeLeCt" all work
```

---

### 4.4 wrap_conditions (Join/Filter Separation)

**Unit Tests**:
```
□ test_wrap_conditions_filter_only - No join candidates
□ test_wrap_conditions_join_only - Only join condition
□ test_wrap_conditions_mixed - Separates joins from filters
□ test_wrap_conditions_nested_and - Handles AND with mixed types
```

---

## Phase 5: Table & Database (Circuit Layer)

### 5.1 Table

**Purpose**: Single table storage with ZSet and row data

**Unit Tests**:
```
□ test_table_new - Creates empty table
□ test_table_reserve - Pre-allocates capacity
□ test_table_apply_mutation_create - Inserts row, adds to ZSet with weight +1
□ test_table_apply_mutation_update - Updates row, ZSet weight unchanged (0)
□ test_table_apply_mutation_delete - Removes row, ZSet weight -1
□ test_table_apply_mutation_double_create - Weight becomes 2
□ test_table_apply_delta - Merges ZSet delta correctly
□ test_table_get_record_version - Extracts _spooky_version field
□ test_table_zset_key_format - Consistent "table:id" format
```

---

### 5.2 Database

**Purpose**: Collection of tables

**Unit Tests**:
```
□ test_database_new - Creates empty database
□ test_database_ensure_table_creates - Creates table if missing
□ test_database_ensure_table_returns_existing - Doesn't recreate
□ test_database_multiple_tables - Can have many tables
```

---

## Phase 6: View (The Core)

This is the most complex component. Test thoroughly!

### 6.1 View Construction & Initialization

**Unit Tests**:
```
□ test_view_new_simple_scan - Correctly identifies is_simple_scan
□ test_view_new_simple_filter - Correctly identifies is_simple_filter
□ test_view_new_with_params - Stores params as SpookyValue
□ test_view_new_sets_format - Uses provided format or default
□ test_view_new_caches_referenced_tables
□ test_view_new_caches_has_subqueries
□ test_view_initialize_after_deserialize - Recomputes cached flags
□ test_view_is_initialized - Checks if flags are set
□ test_view_serialization_preserves_cache - Cache is serialized
□ test_view_serialization_skips_computed_flags
```

---

### 6.2 View - Fast Paths (Single Record Optimization)

**Unit Tests**:
```
□ test_view_apply_single_create - Adds to cache with weight 1
□ test_view_apply_single_create_idempotent - Re-adding keeps weight at 1
□ test_view_apply_single_delete - Removes from cache
□ test_view_try_fast_single_scan_create - Fast path for simple scan + create
□ test_view_try_fast_single_scan_delete
□ test_view_try_fast_single_filter_matches - Record passes filter
□ test_view_try_fast_single_filter_no_match - Record fails filter
□ test_view_fast_path_skips_complex_views - Returns None for joins/limits
```

---

### 6.3 View - record_matches_view

**Unit Tests**:
```
□ test_record_matches_view_scan_correct_table
□ test_record_matches_view_scan_wrong_table
□ test_record_matches_view_filter_passes
□ test_record_matches_view_filter_fails
□ test_record_matches_view_complex_operator - Returns true (conservative)
```

---

### 6.4 View - process_delta (Single Record)

**Unit Tests**:
```
□ test_process_delta_irrelevant_table - Returns None
□ test_process_delta_create_membership - Calls fast path or batch
□ test_process_delta_delete_membership
□ test_process_delta_content_update_in_view - Notifies of update
□ test_process_delta_content_update_now_matches - Treats as addition
□ test_process_delta_content_update_no_longer_matches - Treats as removal
□ test_process_delta_content_update_not_in_view - Returns None
```

---

### 6.5 View - process_batch (Batch Processing)

**Unit Tests**:
```
□ test_process_batch_first_run_emits_all - Empty cache + data => all Created
□ test_process_batch_no_changes - Returns None
□ test_process_batch_additions_only
□ test_process_batch_removals_only
□ test_process_batch_mixed_changes
□ test_process_batch_content_updates - Detects in-place updates
□ test_process_batch_streaming_format - Only includes changed records
□ test_process_batch_flat_format - Includes all records for hash
□ test_process_batch_updates_last_hash
□ test_process_batch_updates_cache - Cache reflects new state
```

---

### 6.6 View - eval_snapshot (Full Scan)

**Unit Tests**:
```
□ test_eval_snapshot_scan_empty_table - Returns empty ZSet
□ test_eval_snapshot_scan_with_data - Returns table's ZSet (borrowed)
□ test_eval_snapshot_scan_missing_table - Returns empty ZSet
□ test_eval_snapshot_filter_passes_some
□ test_eval_snapshot_filter_passes_none - Empty result
□ test_eval_snapshot_filter_numeric_simd - Uses fast path
□ test_eval_snapshot_project - Passes through input
□ test_eval_snapshot_limit - Respects limit count
□ test_eval_snapshot_limit_with_order - Sorts then limits
□ test_eval_snapshot_join_build_probe - Hash join algorithm
□ test_eval_snapshot_join_no_matches - Empty result
```

---

### 6.7 View - eval_delta_batch (Incremental)

**Unit Tests**:
```
□ test_eval_delta_scan_has_delta - Returns delta for correct table
□ test_eval_delta_scan_no_delta - Returns empty for other tables
□ test_eval_delta_filter_filters_delta - Only passes matching records
□ test_eval_delta_filter_numeric_simd - Uses fast path
□ test_eval_delta_project_no_subquery - Passes through
□ test_eval_delta_project_with_subquery - Returns None (fallback)
□ test_eval_delta_join_returns_none - Cannot do incremental join
□ test_eval_delta_limit_returns_none - Cannot do incremental limit
```

---

### 6.8 View - Subquery Handling

**Unit Tests**:
```
□ test_view_has_subqueries_true
□ test_view_has_subqueries_false
□ test_expand_with_subqueries_no_subqueries - No-op
□ test_expand_with_subqueries_adds_results - Subquery records added
□ test_expand_with_subqueries_normalizes - All weights = 1
□ test_evaluate_subqueries_for_parent_into - Recursive evaluation
□ test_subquery_with_param_context - $parent.field resolution
```

---

### 6.9 View - categorize_changes

**Unit Tests**:
```
□ test_categorize_entering_view - was_member=false, will_be=true => addition
□ test_categorize_leaving_view - was_member=true, will_be=false => removal
□ test_categorize_staying_in - was_member=true, will_be=true => no change
□ test_categorize_staying_out - was_member=false, will_be=false => no change
□ test_categorize_content_updates_for_members - Only if still member
□ test_categorize_content_updates_excludes_removals
```

---

### 6.10 View - get_row_value

**Unit Tests**:
```
□ test_get_row_value_raw_id - Finds "123" in rows
□ test_get_row_value_prefixed_id - Finds "table:123" in rows
□ test_get_row_value_fallback - Tries both formats
□ test_get_row_value_missing - Returns None
□ test_get_row_value_invalid_key_format - Returns None
```

---

## Phase 7: Circuit (Orchestration)

### 7.1 Circuit Construction & Management

**Unit Tests**:
```
□ test_circuit_new - Creates empty circuit
□ test_circuit_with_capacity - Pre-allocates
□ test_circuit_register_view - Adds view and returns initial update
□ test_circuit_register_view_duplicate - Replaces existing
□ test_circuit_unregister_view - Removes by ID
□ test_circuit_rebuild_dependency_list - Correct table->view mapping
□ test_circuit_ensure_dependency_list - Lazy initialization
```

---

### 7.2 Circuit - ingest_single

**Unit Tests**:
```
□ test_ingest_single_no_views - Returns empty
□ test_ingest_single_irrelevant_table - Returns empty
□ test_ingest_single_create_affects_view - Returns update
□ test_ingest_single_update_affects_view
□ test_ingest_single_delete_affects_view
□ test_ingest_single_multiple_views - Returns all affected
```

---

### 7.3 Circuit - ingest_batch

**Unit Tests**:
```
□ test_ingest_batch_empty - Returns empty
□ test_ingest_batch_single_table - Processes correctly
□ test_ingest_batch_multiple_tables - Groups by table
□ test_ingest_batch_mixed_operations - Create/Update/Delete in one batch
□ test_ingest_batch_no_affected_views - Returns empty
□ test_ingest_batch_propagates_to_views - All relevant views notified
```

---

### 7.4 Circuit - init_load

**Unit Tests**:
```
□ test_init_load_single_record - Adds to table
□ test_init_load_multiple_records - All added
□ test_init_load_creates_tables - Ensures tables exist
□ test_init_load_grouped - Efficient grouped loading
□ test_init_load_reserves_capacity
```

---

## Phase 8: Update Formatting (Output Layer)

### 8.1 ViewResultFormat & ViewUpdate

**Unit Tests**:
```
□ test_view_result_format_default - Flat
□ test_view_result_format_copy - Is Copy trait
□ test_view_update_query_id
□ test_view_update_hash_flat - Returns hash
□ test_view_update_hash_streaming - Returns None
□ test_view_update_has_streaming_changes
□ test_view_update_record_count
```

---

### 8.2 ViewDelta

**Unit Tests**:
```
□ test_view_delta_empty
□ test_view_delta_additions_only
□ test_view_delta_removals_only
□ test_view_delta_updates_only
□ test_view_delta_is_empty
□ test_view_delta_len
```

---

### 8.3 compute_flat_hash

**Unit Tests**:
```
□ test_compute_flat_hash_empty - Consistent empty hash
□ test_compute_flat_hash_deterministic - Same input => same hash
□ test_compute_flat_hash_order_independent - [a,b] == [b,a] in hash
□ test_compute_flat_hash_different_content - Different hash for different content
```

---

### 8.4 build_update

**Unit Tests**:
```
□ test_build_update_flat - Creates MaterializedViewUpdate
□ test_build_update_tree - Creates Tree variant
□ test_build_update_streaming_with_delta - Uses delta info
□ test_build_update_streaming_no_delta - Treats all as Created
□ test_build_update_with_hash - Uses precomputed hash
□ test_build_streaming_delta - Converts (id, weight) to DeltaRecords
```

---

## Phase 9: Integration Tests

### 9.1 Module: View Processing Pipeline

**Integration Tests**:
```
□ test_view_full_lifecycle - Create view, process changes, verify output
□ test_view_multiple_tables - View depending on 2+ tables
□ test_view_with_subquery_lifecycle - Parent + child records
□ test_view_format_switching - Same data, different formats
```

---

### 9.2 Module: Circuit Processing Pipeline

**Integration Tests**:
```
□ test_circuit_full_lifecycle - Load data, register views, ingest changes
□ test_circuit_multiple_views_same_table - All receive updates
□ test_circuit_cascading_changes - Changes propagate correctly
□ test_circuit_parallel_safe - (if feature=parallel) concurrent safety
```

---

### 9.3 Module: Converter Pipeline

**Integration Tests**:
```
□ test_sql_to_view_simple - SQL -> QueryPlan -> View -> process
□ test_sql_to_view_complex - Subqueries, joins, limits
□ test_sql_to_view_with_params - Parameter substitution
```

---

## Phase 10: End-to-End Pipeline Tests

**E2E Tests**:
```
□ test_e2e_crud_flow
  - init_load initial data
  - Register view
  - Create record -> view notified with Created
  - Update record -> view notified with Updated
  - Delete record -> view notified with Deleted

□ test_e2e_filter_changes_membership
  - Record initially matches filter
  - Update makes it not match -> Deleted event
  - Update makes it match again -> Created event

□ test_e2e_subquery_parent_child
  - Load users and posts
  - View: SELECT *, (SELECT ... FROM user) AS author FROM post
  - Create post -> both post and author in view
  - Delete author -> view still has post (author resolved is stale or null)

□ test_e2e_join_updates
  - Two tables with join relationship
  - Insert matching records -> appear in view
  - Delete one side -> disappear from view

□ test_e2e_limit_order_stability
  - Table with 100 records
  - View: LIMIT 10 ORDER BY created_at DESC
  - Insert older record -> no change (outside top 10)
  - Insert newest record -> bumps oldest from top 10

□ test_e2e_streaming_vs_flat_consistency
  - Same operations, both formats
  - Verify equivalent final state

□ test_e2e_batch_vs_single_consistency
  - Same records via ingest_single vs ingest_batch
  - Verify identical view state
```

---

## Implementation Order Summary

| Order | Phase | Focus | Est. Tests |
|-------|-------|-------|------------|
| 1 | Path | Field path traversal | 7 |
| 2 | SpookyValue | Data representation | 15 |
| 3 | ZSet Types | Weighted multisets | 30 |
| 4 | Operation/Delta | Mutation types | 12 |
| 5 | BatchDeltas | Batch grouping | 7 |
| 6 | Eval Helpers | resolve_nested, compare, hash | 25 |
| 7 | Predicate | WHERE conditions | 18 |
| 8 | Operator | Query plan nodes | 15 |
| 9 | Projection/Order | Query components | 6 |
| 10 | SQL Parser | convert_surql_to_dbsp | 20 |
| 11 | Table | Storage layer | 9 |
| 12 | Database | Table collection | 4 |
| 13 | View Construction | View setup | 10 |
| 14 | View Fast Paths | Single record opt | 9 |
| 15 | View process_delta | Single record processing | 7 |
| 16 | View process_batch | Batch processing | 10 |
| 17 | View eval_snapshot | Full scan | 12 |
| 18 | View eval_delta | Incremental | 8 |
| 19 | View Subqueries | Nested queries | 6 |
| 20 | View categorize | Change classification | 6 |
| 21 | Circuit Mgmt | View registration | 6 |
| 22 | Circuit Ingest | Data processing | 10 |
| 23 | Update Format | Output building | 15 |
| 24 | View Integration | Module tests | 4 |
| 25 | Circuit Integration | Module tests | 4 |
| 26 | Converter Integration | Module tests | 3 |
| 27 | E2E Pipeline | Full system tests | 8 |

**Total: ~286 tests**

---

## Quick Reference: Test File Structure

```
tests/
├── unit/
│   ├── types/
│   │   ├── path_tests.rs
│   │   ├── spooky_value_tests.rs
│   │   ├── zset_tests.rs
│   │   ├── operation_tests.rs
│   │   └── batch_deltas_tests.rs
│   ├── eval/
│   │   ├── resolve_tests.rs
│   │   ├── compare_tests.rs
│   │   ├── hash_tests.rs
│   │   └── numeric_filter_tests.rs
│   ├── operators/
│   │   ├── predicate_tests.rs
│   │   ├── operator_tests.rs
│   │   └── projection_tests.rs
│   ├── converter/
│   │   ├── parser_tests.rs
│   │   └── full_query_tests.rs
│   ├── circuit/
│   │   ├── table_tests.rs
│   │   └── database_tests.rs
│   ├── view/
│   │   ├── construction_tests.rs
│   │   ├── fast_path_tests.rs
│   │   ├── process_delta_tests.rs
│   │   ├── process_batch_tests.rs
│   │   ├── eval_snapshot_tests.rs
│   │   ├── eval_delta_tests.rs
│   │   └── subquery_tests.rs
│   └── update/
│       ├── format_tests.rs
│       └── build_tests.rs
├── integration/
│   ├── view_pipeline_tests.rs
│   ├── circuit_pipeline_tests.rs
│   └── converter_pipeline_tests.rs
└── e2e/
    └── full_pipeline_tests.rs
```

---

## Notes

1. **Existing tests**: Your file already has some tests (ZSet, Table, View). Build on those.
2. **Feature flags**: Remember to test both `parallel` and non-parallel paths where applicable.
3. **WASM**: Some tests may need `#[cfg(not(target_arch = "wasm32"))]` guards.
4. **Tracing**: Consider using `tracing-test` for verifying log output in complex scenarios.
