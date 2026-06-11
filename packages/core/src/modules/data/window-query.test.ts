import { describe, it, expect } from 'vitest';
import { buildWindowMaterialization } from './window-query';

describe('buildWindowMaterialization', () => {
  it('rewrites the game-list window query (START 30) to select the id-set, keeping ORDER BY', () => {
    const surql =
      'SELECT * FROM game WHERE database = $database ORDER BY sort_index asc, date desc LIMIT 30 START 30;';
    const r = buildWindowMaterialization(surql);
    expect(r).not.toBeNull();
    expect(r!.query).toBe('SELECT * FROM $__win ORDER BY sort_index asc, date desc');
  });

  it('returns null for START 0 (offset-free windows still work via the normal re-query)', () => {
    const surql =
      'SELECT * FROM game WHERE database = $database ORDER BY sort_index asc, date desc LIMIT 30 START 0;';
    expect(buildWindowMaterialization(surql)).toBeNull();
  });

  it('returns null when there is no START clause', () => {
    expect(
      buildWindowMaterialization('SELECT * FROM game WHERE database = $database LIMIT 30;')
    ).toBeNull();
    expect(buildWindowMaterialization('SELECT * FROM game;')).toBeNull();
  });

  it('preserves a custom projection', () => {
    const surql = 'SELECT id, white, black FROM game ORDER BY date desc LIMIT 30 START 60;';
    expect(buildWindowMaterialization(surql)!.query).toBe(
      'SELECT id, white, black FROM $__win ORDER BY date desc'
    );
  });

  it('omits ORDER BY when the query had none', () => {
    const surql = 'SELECT * FROM game LIMIT 30 START 30;';
    expect(buildWindowMaterialization(surql)!.query).toBe('SELECT * FROM $__win');
  });

  it('ignores FROM/ORDER BY/LIMIT/START inside subqueries (paren-aware)', () => {
    const surql =
      'SELECT *, (SELECT * FROM comment WHERE game = $parent.id ORDER BY created_at desc LIMIT 5) AS comments ' +
      'FROM game ORDER BY sort_index asc LIMIT 30 START 90;';
    expect(buildWindowMaterialization(surql)!.query).toBe(
      'SELECT *, (SELECT * FROM comment WHERE game = $parent.id ORDER BY created_at desc LIMIT 5) AS comments FROM $__win ORDER BY sort_index asc'
    );
  });

  it('does not treat a START inside a string literal as the offset', () => {
    const surql = "SELECT * FROM game WHERE note = 'LIMIT 30 START 30' ORDER BY date desc LIMIT 30 START 30;";
    const r = buildWindowMaterialization(surql);
    expect(r!.query).toBe('SELECT * FROM $__win ORDER BY date desc');
  });
});
