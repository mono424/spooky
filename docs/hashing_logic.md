# Spooky Hashing Logic Documentation

## Overview

The Spooky Hashing System provides a robust, Merkle Tree-like mechanism for tracking data state and dependencies in SurrealDB. It enables efficient synchronization and change detection by ensuring that any change to a record, its dependencies, or the records it references is reflected in a deterministic hash.

The system relies on a shadow table, `_spooky_data_hash`, which maintains hash data for every record in the database.

## Architecture

For every tracked record `table:id`, there is a corresponding `_spooky_data_hash` record with `RecordId = table:id`. This record contains three primary hash components:

1.  **IntrinsicHash**: Representative of the record's own scalar data.
2.  **CompositionHash**: Representative of the record's relationships (outgoing references and incoming dependencies).
3.  **TotalHash**: The final, aggregate hash representing the complete state (`Intrinsic XOR Composition`).

### 1. IntrinsicHash

The `IntrinsicHash` represents the "local" state of the record.

- **Content**: XOR sum of the hashes of all **scalar fields** (strings, numbers, booleans, dates) in the record.
- **Exclusions**:
  - Reference fields (e.g., `author: user:123`) are **excluded** here and handled in `CompositionHash`.
  - System fields (`id`) are implicitly part of the record identity but not hashed into IntrinsicHash to allow for content-addressable logic if needed.
- **Update Trigger**: Updates whenever a scalar field on the record changes.

### 2. CompositionHash

The `CompositionHash` represents the "relational" state. It is an Object containing sub-hashes for specific relationships, plus an aggregate XOR sum.

Structure:

```json
CompositionHash: {
    "author": "hash_of_referenced_user",
    "comments": "hash_of_all_child_comments",
    "_xor": "aggregate_xor_of_all_values_above"
}
```

It comprises two types of relationships:

#### A. Outgoing References (Reference Propagation)

When a record points to another (e.g., `thread.author -> user`), the **TotalHash** of the referenced record is included in the referrer's `CompositionHash`.

- **Mechanism**:
  - If `thread:A` has `author = user:B`, then `CompositionHash.author` = `user:B.TotalHash`.
  - **Cascade Down**: If `user:B` changes, a trigger (`_spooky_z_cascade`) "touches" `thread:A` (updating `_spooky_touch` timestamp). This forces `thread:A` to re-read `user:B`'s new hash and update itself.
- **Cycle Breaking**:
  - To prevent infinite feedback loops (e.g., Child hashes Parent <-> Parent hashes Child), **Referenced Fields that point to a Parent are excluded** from the Hashing logic on the Child side.

#### B. Incoming Dependencies (Bubble Up)

When a record is a parent to others (e.g., `Thread` has many `Comments`), the aggregate hash of all children is included.

- **Mechanism**:
  - When `comment:X` (child) changes, it calculates the "Delta" of its own TotalHash.
  - It then "Bubbles Up" this Delta to the parent `thread:Y`, XORing it into `CompositionHash.comment` and `CompositionHash._xor`.
- **Result**: The Parent's hash effectively includes the state of all its children.

### 3. TotalHash

The definitive state identifier for the record.

```rust
TotalHash = IntrinsicHash XOR CompositionHash._xor
```

## Propagation Mechanics

The system ensures consistency through two propagation flows:

### 1. Bubble Up (Child -> Parent)

_Used for: One-to-Many dependencies (e.g., Thread -> Comments)._

1.  **Mutation**: `Comment` is updated.
2.  **Calculation**: New `TotalHash` for Comment is generated.
3.  **Delta**: Difference between Old and New Hash is calculated (`Old XOR New`).
4.  **Propagation**: The Delta is XORed into the Parent's `CompositionHash`.
    - `UPDATE _spooky_data_hash SET CompositionHash.comment = XOR(..., Delta) WHERE RecordId = parent_id`.

### 2. Cascade Down / Reference Propagation (Referenced -> Referrer)

_Used for: Foreign Keys / References (e.g., Thread -> Author)._

1.  **Mutation**: `User` (Author) is updated.
2.  **Trigger**: `DEFINE EVENT _spooky_z_cascade_thread_author` fires on `User`.
3.  **Action**: Updates a hidden field `_spooky_touch` on all `Thread` records referencing this User.
    - `UPDATE thread SET _spooky_touch = time::now() WHERE author = $after.id`.
4.  **Reaction**: The update to `Thread` triggers its own Mutation Event.
5.  **Re-Evaluation**: `Thread` re-calculates its `CompositionHash`. It sees the new `TotalHash` of the User and incorporates it.

## Cycle Prevention

A critical component is preventing Hashing Cycles. If `Thread` hashes `User` (Reference), and `User` hashes `Thread` (Dependency) – or if `Comment` hashes `Thread` (Reference) and `Thread` hashes `Comment` (Dependency) – any change creates an infinite loop or "dirty" state where reverting a change doesn't restore the original hash.

**Solution**:

- The system identifies the "Parent" relationship (defined in schema or inferred).
- **Rule**: A Child record does **NOT** include the hash of its Parent reference in its `CompositionHash`.
- This ensures the Dependency Graph is a Directed Acyclic Graph (DAG) for hashing purposes.

## Reversibility

Because all aggregations use **XOR** (Reducible), the system is fully reversible.

- Adding a record applies `HASH`.
- Removing the record applies `HASH` again (which cancels it out: `A XOR A = 0`).
- Modifying uses `Delta` (`Old XOR New`). Applying `Delta` transforms `State_Old` to `State_New`.
- Reverting the modification applies the inverse Delta, restoring `State_Old` exactly.

## Example Flow

**Scenario**: User updates their Avatar.

1.  **User Table**:

    - `IntrinsicHash` changes (due to `avatar` field).
    - `TotalHash` changes.
    - _Event_: `_spooky_z_cascade` fires for `Thread` table.

2.  **Thread Table** (Referencing User):

    - `_spooky_touch` updated.
    - Thread Mutation Event fires.
    - `CompositionHash` recalculates: Reads new `User.TotalHash`.
    - `TotalHash` of Thread changes.

3.  **System**:
    - Sync clients see `Thread` has changed (new version).
    - They fetch `Thread` and see `author` has changed.
