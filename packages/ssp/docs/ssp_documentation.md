# SSP Module Documentation

## Module: `ssp::engine::operators::operator`

Defines the core Abstract Syntax Tree (AST) for the Query Engine. The `Operator` enum represents the logical nodes of a query plan, forming a tree structure that describes data transformations.

### 1. Enum `Operator`

**Type**: `Recursive Enum`
**Derives**: `Serialize`, `Deserialize`, `Clone`, `Debug`
**Serialization**: Tagged "op", all fields lowercase.

This enum is the building block of `QueryPlan`.

#### Variants

##### 1.1 `Scan`
Represents the source of data. It reads all records from a physical table.

*   **Structure**:
    ```rust
    Scan {
        table: String, // Name of the physical table to read from
    }
    ```
*   **Usage**: Always the leaf node of an operator tree.
*   **Complexity Analysis**:
    *   **AST Traversal**: **O(1)**.
    *   **Execution (Snapshot)**: **O(N)** where N is the total number of rows in the table. It acts as a generator.
    *   **Execution (Incremental)**: **O(1)**. In dependency graph logic, a Scan is essentially a pass-through for incoming table deltas.

##### 1.2 `Filter`
Applies a boolean predicate to filter records.

*   **Structure**:
    ```rust
    Filter {
        input: Box<Operator>, // Upstream operator
        predicate: Predicate, // Boolean logic expression
    }
    ```
*   **Usage**: Reduces the dataset based on conditions (e.g., `WHERE id = "123"`).
*   **Complexity Analysis**:
    *   **AST Traversal**: **O(T)** to traverse input tree.
    *   **Execution (Snapshot)**: **O(N * P)** where N is input rows and P is predicate complexity.
    *   **Execution (Incremental)**: **O(D * P)** where D is the size of the incoming batch delta. Filter is stateless.

##### 1.3 `Join`
Performs an equi-join between two inputs.

*   **Structure**:
    ```rust
    Join {
        left: Box<Operator>,
        right: Box<Operator>,
        on: JoinCondition, // Fields to join on
    }
    ```
*   **Usage**: Combines data from two sources (e.g., `FROM user JOIN comment ON user.id = comment.author`).
*   **Complexity Analysis**:
    *   **AST Traversal**: **O(L + R)** nodes.
    *   **Execution (Snapshot)**: **O(R + L)**.
        *   Build Phase: **O(R)** to build hash index of Right side.
        *   Probe Phase: **O(L)** to iterate Left side and look up matches.
    *   **Execution (Incremental)**: **O(D_left + D_right)**?
        *   *Correction*: Standard join logic typically typically requires maintaining state for both sides. If utilizing simple nested loop or stateless join (impossible for general streams), cost is higher. In SSP `process_batch`, joins typically trigger a **Full Re-evaluation (Fallback)** which is **O(Total_Rows)**. Incremental joins are not yet fully implemented in the fast path.

##### 1.4 `Project`
Transforms records by selecting fields or executing subqueries.

*   **Structure**:
    ```rust
    Project {
        input: Box<Operator>,
        projections: Vec<Projection>, // List of columns/subqueries
    }
    ```
*   **Usage**: `SELECT name, (SELECT count(*) ...) FROM ...`
*   **Complexity Analysis**:
    *   **AST Traversal**: **O(Input + Projections)**.
    *   **Execution (Snapshot)**: **O(N * S)**.
        *   For simple field selection: **O(N)**.
        *   If `projections` contains `Subquery`: **O(N * Cost_Subquery)**. This is the most expensive operation type ("N+1 Query Problem").
    *   **Execution (Incremental)**:
        *   Simple Project: **O(D)**.
        *   Subquery Project: typically triggers **Fallback** or complex re-eval.

##### 1.5 `Limit`
Truncates the result set, optionally sorting it first.

*   **Structure**:
    ```rust
    Limit {
        input: Box<Operator>,
        limit: usize,
        #[serde(default)]
        order_by: Option<Vec<OrderSpec>>, // Optional sorting
    }
    ```
*   **Usage**: `LIMIT 10` or `ORDER BY created_at DESC LIMIT 5`.
*   **Complexity Analysis**:
    *   **AST Traversal**: **O(Input)**.
    *   **Execution (Snapshot)**:
        *   Without Sort: **O(N)** (single pass, stop after limit).
        *   With Sort: **O(N log N)**. Must consume ALL input, sort it, then take `limit`.
    *   **Execution (Incremental)**:
        *   Changes to the input set (Additions/Removals) usually require re-evaluating the top-k window.
        *   **Cost**: **O(N log N)** (Fallback). Differential maintenance of finding the "next best" record after a deletion requires full re-sort or advanced structures (Heap) not currently fully optimized in incremental path.

---

### 2. Impl `Operator`

Methods defined directly on the `Operator` struct.

#### 2.1 `fn referenced_tables(&self) -> Vec<String>`

Extracts a deduplicated list of all table names referenced anywhere in this operator tree.

*   **Logic**:
    1.  Initialize empty `tables` vector.
    2.  Call `collect_referenced_tables_recursive`.
    3.  Create a `HashSet` to filter duplicates while retaining first-occurrence order (stable-ish dedupe).
    4.  Return vector.
*   **Usage**: Used during View Registration to build the dependency graph (`Circuit::dependency_list`).
*   **Complexity**:
    *   **Time**: **O(N + T)** where N is number of nodes in tree (traversal) and T is number of tables found (deduplication overhead).
    *   **Space**: **O(T)** to store table names.

#### 2.2 `fn collect_referenced_tables_recursive(&self, tables: &mut Vec<String>)`

Internal helper for recursive traversal.

*   **Logic**:
    *   **Scan**: Adds `table` string to list.
    *   **Filter/Limit**: Recurses into `input`.
    *   **Join**: Recurses into `left` then `right`.
    *   **Project**: Recurses into `input`, then iterates `projections`. If a projection is a `Subquery`, recurses into the subquery's `plan`.
*   **Complexity**: **O(N)** where N is the total count of Operator nodes + Projection nodes in the hierarchy.

#### 2.3 `fn has_subquery_projections(&self) -> bool`

Checks if this operator or any of its children contains a Subquery projection.

*   **Logic**:
    *   **Scan**: Returns `false`.
    *   **Filter/Limit**: Delegates to `input.has_subquery_projections()`.
    *   **Join**: Returns `true` if `left` OR `right` has subqueries.
    *   **Project**:
        1.  Checks if any item in `projections` is `Projection::Subquery`. If yes, returns `true` (Local check).
        2.  If no, recurses into `input.has_subquery_projections()`.
*   **Usage**: Used to set the `has_subqueries_cached` flag on the View. Views with subqueries require significantly more expensive processing logic (`expand_with_subqueries`).
*   **Complexity**: **O(N)** traversal. Returns early (short-circuit) if a subquery is found.
