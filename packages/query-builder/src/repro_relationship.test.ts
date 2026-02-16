import { describe, it, expect } from 'vitest';
import { QueryBuilder, SchemaStructure } from './index';

describe('QueryBuilder Relationship Inference', () => {
  const schema = {
    tables: [
      {
        name: 'game_database',
        columns: {
          id: { type: 'string', recordId: true, optional: false },
          games: { type: 'string', optional: true },
        },
        primaryKey: ['id'],
      },
      {
        name: 'game',
        columns: {
          id: { type: 'string', recordId: true, optional: false },
          database: { type: 'string', recordId: true, optional: false },
        },
        primaryKey: ['id'],
      },
    ],
    relationships: [
      {
        from: 'game_database',
        field: 'games',
        to: 'game',
        cardinality: 'many',
      },
      {
        from: 'game',
        field: 'database',
        to: 'game_database',
        cardinality: 'one',
      },
    ],
    backends: {},
  } as const;

  it('should correctly infer foreign key field for game_database -> games', () => {
    const qb = new QueryBuilder(schema, 'game_database');

    // We want to simulate: SELECT *, (SELECT * FROM game WHERE database=$parent.id) AS games FROM game_database
    // This happens when we include 'games' in the related fields

    // The QueryBuilder.related method adds to options.related
    qb.related('games');

    // To verify the generated subquery, we need to inspect the build output
    // buildQueryFromOptions is internal, but we can check the result of build()
    // or we can inspect the private options if we cast to any

    const options = (qb as any).options;
    expect(options.related).toHaveLength(1);
    expect(options.related[0].alias).toBe('games');
    expect(options.related[0].foreignKeyField).toBe('database');
  });
});
