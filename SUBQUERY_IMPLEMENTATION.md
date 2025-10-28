# Subquery Implementation for Related Data

## Overview

The related feature has been successfully implemented using **subqueries** instead of the `FETCH` clause. This provides a more flexible and SQL-standard approach to loading related data.

## Changes Made

### 1. Updated Interfaces ([table-queries.ts:6-25](packages/client-solid/src/lib/table-queries.ts#L6-L25))

```typescript
export interface RelatedQuery {
  /** The name of the related table to query */
  relatedTable: string;
  /** The alias for this subquery result (defaults to relatedTable name) */
  alias?: string;
}

export interface LiveQueryOptions<Model extends GenericModel> {
  select?: ((keyof Model & string) | "*")[];
  where?: Partial<Model> | { id: RecordId };
  limit?: number;
  offset?: number;
  /** Related tables to include via subqueries */
  related?: RelatedQuery[];
}
```

**Removed**: The old `fetch?: string[]` property

### 2. Updated `related()` Method ([table-queries.ts:300-326](packages/client-solid/src/lib/table-queries.ts#L300-L326))

The `related()` method now tracks subquery relationships instead of FETCH fields:

```typescript
related<RelatedTable extends string>(relatedTableOrAlias: RelatedTable): this {
  if (!this.options.related) {
    this.options.related = [];
  }

  // Check if this relationship already exists
  const exists = this.options.related.some(
    r => (r.alias || r.relatedTable) === relatedTableOrAlias
  );

  if (!exists) {
    this.options.related.push({
      relatedTable: relatedTableOrAlias,
      alias: relatedTableOrAlias,
    });
  }
  return this;
}
```

### 3. Added `generateSubquery()` Method ([table-queries.ts:368-397](packages/client-solid/src/lib/table-queries.ts#L368-L397))

This method generates the SQL subquery based on naming conventions:

```typescript
private generateSubquery(currentTable: string, relatedAlias: string): string {
  // Handle pluralization to get the actual table name
  let relatedTable = relatedAlias;

  // Simple pluralization: remove trailing 's' to get singular form
  if (relatedAlias.endsWith('s') && relatedAlias.length > 1) {
    relatedTable = relatedAlias.slice(0, -1);
  }

  // The foreign key is the name of the current table
  const foreignKey = currentTable;

  // Build the subquery using $parent.id
  return `(SELECT * FROM ${relatedTable} WHERE ${foreignKey}=$parent.id)`;
}
```

### 4. Updated `buildQuery()` Method ([table-queries.ts:399-447](packages/client-solid/src/lib/table-queries.ts#L399-L447))

The query builder now generates subqueries instead of FETCH clauses:

```typescript
private buildQuery(method: "LIVE SELECT" | "SELECT", props: LiveQueryOptions<Model>): QueryInfo {
  // Build the select clause with subqueries for related data
  let selectParts = props.select ?? ["*"];
  const selectFields = selectParts.map((key) => `${key}`).join(", ");

  // Add subqueries for related tables
  const subqueries: string[] = [];
  if (props.related && props.related.length > 0) {
    for (const rel of props.related) {
      const subquery = this.generateSubquery(this.tableName, rel.alias || rel.relatedTable);
      const alias = rel.alias || rel.relatedTable;
      subqueries.push(`${subquery} AS ${alias}`);
    }
  }

  // Combine regular fields and subqueries
  const allSelectParts = [selectFields, ...subqueries].join(", ");

  let query = `${method} ${allSelectParts} FROM ${this.tableName}`;
  // ... rest of query building
}
```

## Usage Example

The API remains the same for users:

```typescript
// In ThreadDetail component
const threadQuery = db.query.thread
  .find({ id: new RecordId("thread", params.id) })
  .related("comments")  // ← Same API, but uses subqueries internally
  .query();
```

## Generated SQL

### Before (FETCH approach)
```sql
SELECT * FROM thread WHERE id = $id FETCH comments;
```

### After (Subquery approach)
```sql
SELECT *, (SELECT * FROM comment WHERE thread=$parent.id) AS comments
FROM thread
WHERE id = $id;
```

## Benefits

1. **More flexible**: Doesn't require predefined REFERENCE fields in the schema
2. **SQL standard**: Uses standard subquery syntax that's widely understood
3. **Dynamic**: Can query any relationship without schema modifications
4. **Maintains live updates**: Works with both `SELECT` and `LIVE SELECT` queries
5. **Better for complex queries**: Subqueries can be extended with additional WHERE clauses, ordering, etc.

## Schema Inference

The implementation uses simple naming conventions:

- **Plural to singular**: "comments" → "comment" table
- **Foreign key inference**: For table "thread", related records should have a "thread" field
- **$parent.id reference**: Uses SurrealDB's `$parent` variable to reference the parent row's ID

## Testing

The implementation has been:
- ✅ Built successfully without TypeScript errors
- ✅ Compatible with the existing `ThreadDetail` component
- ✅ Tested with both regular and live queries
- ✅ Maintains the same public API for backward compatibility

## Future Improvements

1. **Schema metadata**: Could use the generated RELATIONSHIPS constant at runtime for smarter inference
2. **Custom foreign keys**: Allow specifying custom foreign key fields
3. **Nested subqueries**: Support multiple levels of related data
4. **Performance optimizations**: Cache subquery generation results
