import { describe, it, expect } from 'vitest';
import type { QueryPlan } from '@spooky-sync/query-builder';
import { renderBaseSelectSurql, renderRelationFetchSurql } from './plan-render';
import { buildWindowMaterializationPlan } from '../../modules/data/window-query';

describe('renderBaseSelectSurql', () => {
  it('renders projection, where, order, limit, offset', () => {
    const plan: QueryPlan = {
      table: 'post',
      select: ['title', 'body'],
      where: [{ field: 'published', op: '=', value: true }],
      orderBy: [['created', 'desc']],
      limit: 10,
      offset: 20,
    };
    const { sql, vars } = renderBaseSelectSurql(plan);
    expect(sql).toBe('SELECT title, body FROM post WHERE published = $__p0 ORDER BY created desc LIMIT 10 START 20;');
    expect(vars).toEqual({ __p0: true });
  });

  it('renders OR groups and comparison ops', () => {
    const plan: QueryPlan = {
      table: 'game',
      where: [
        { or: [{ field: 'white', op: '=', value: 'u:1' }, { field: 'black', op: '=', value: 'u:1' }] },
        { field: 'moves', op: '>=', value: 5 },
      ],
    };
    const { sql, vars } = renderBaseSelectSurql(plan);
    expect(sql).toBe('SELECT * FROM game WHERE (white = $__p0 OR black = $__p1) AND moves >= $__p2;');
    expect(vars).toEqual({ __p0: 'u:1', __p1: 'u:1', __p2: 5 });
  });

  it('honors paramRef and swap without binding a new var', () => {
    const plan: QueryPlan = {
      table: 't',
      where: [{ field: 'tags', op: 'CONTAINS', value: undefined, paramRef: 'tag', swap: true }],
    };
    const { sql, vars } = renderBaseSelectSurql(plan, { tag: 'x' });
    expect(sql).toBe('SELECT * FROM t WHERE $tag CONTAINS tags;');
    expect(vars).toEqual({ tag: 'x' });
  });
});

describe('renderRelationFetchSurql', () => {
  it('builds a WHERE ... IN $__keys batch fetch, omitting LIMIT', () => {
    const { sql, vars } = renderRelationFetchSurql({
      table: 'comment',
      matchField: 'post',
      keys: ['post:1', 'post:2'],
      where: [{ field: 'hidden', op: '=', value: false }],
      orderBy: [['rank', 'asc']],
    });
    expect(sql).toBe('SELECT * FROM comment WHERE post IN $__keys AND hidden = $__p0 ORDER BY rank asc;');
    expect(vars).toEqual({ __keys: ['post:1', 'post:2'], __p0: false });
  });
});

describe('buildWindowMaterializationPlan', () => {
  it('restricts to the id-set for offset queries, dropping where/limit/offset', () => {
    const plan: QueryPlan = {
      table: 'post',
      where: [{ field: 'x', op: '=', value: 1 }],
      orderBy: [['created', 'desc']],
      limit: 10,
      offset: 20,
      relations: [{ alias: 'a', table: 'u', cardinality: 'one', foreignKeyField: 'a' }],
    };
    const win = buildWindowMaterializationPlan(plan, ['post:5', 'post:6']);
    expect(win).toEqual({
      table: 'post',
      orderBy: [['created', 'desc']],
      relations: [{ alias: 'a', table: 'u', cardinality: 'one', foreignKeyField: 'a' }],
      ids: ['post:5', 'post:6'],
      where: undefined,
      limit: undefined,
      offset: undefined,
    });
  });

  it('returns null for non-offset queries (keep normal path)', () => {
    expect(buildWindowMaterializationPlan({ table: 'post', limit: 10 }, ['post:1'])).toBeNull();
    expect(buildWindowMaterializationPlan({ table: 'post', offset: 0 }, ['post:1'])).toBeNull();
  });
});
