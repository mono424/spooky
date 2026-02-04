import { MutationEventType } from '../types.js';

export const surql = {
  seal(query: string) {
    return `${query};`;
  },

  tx(queries: string[]) {
    return `BEGIN TRANSACTION;${queries.join(';')};COMMIT TRANSACTION`;
  },

  selectById(idVar: string, returnValues: string[]) {
    return `SELECT ${returnValues.join(',')} FROM ONLY $${idVar}`;
  },

  selectByFieldsAnd(
    table: string,
    whereVar: ({ field: string; variable: string } | string)[],
    returnValues: ({ field: string; alias: string } | string)[]
  ) {
    return `SELECT ${returnValues
      .map((returnValues) =>
        typeof returnValues === 'string'
          ? returnValues
          : `${returnValues.field} as ${returnValues.alias}`
      )
      .join(',')} FROM ${table} WHERE ${whereVar
      .map((whereVar) =>
        typeof whereVar === 'string'
          ? `${whereVar} = $${whereVar}`
          : `${whereVar.field} = $${whereVar.variable}`
      )
      .join(' AND ')}`;
  },

  create(idVar: string, dataVar: string) {
    return `CREATE ONLY $${idVar} CONTENT $${dataVar}`;
  },

  createSet(
    idVar: string,
    keyDataVars: ({ key: string; variable: string } | { statement: string } | string)[]
  ) {
    return `CREATE ONLY $${idVar} SET ${keyDataVars
      .map((keyDataVar) =>
        typeof keyDataVar === 'string'
          ? `${keyDataVar} = $${keyDataVar}`
          : 'statement' in keyDataVar
            ? keyDataVar.statement
            : `${keyDataVar.key} = $${keyDataVar.variable}`
      )
      .join(', ')}`;
  },

  upsert(idVar: string, dataVar: string) {
    return `UPSERT ONLY $${idVar} REPLACE $${dataVar}`;
  },

  updateMerge(idVar: string, dataVar: string) {
    return `UPDATE ONLY $${idVar} MERGE $${dataVar}`;
  },

  updateSet(
    idVar: string,
    keyDataVar: ({ key: string; variable: string } | { statement: string } | string)[]
  ) {
    return `UPDATE $${idVar} SET ${keyDataVar
      .map((keyDataVar) =>
        typeof keyDataVar === 'string'
          ? `${keyDataVar} = $${keyDataVar}`
          : 'statement' in keyDataVar
            ? keyDataVar.statement
            : `${keyDataVar.key} = $${keyDataVar.variable}`
      )
      .join(', ')}`;
  },

  delete(idVar: string) {
    return `DELETE $${idVar}`;
  },

  let(name: string, query: string) {
    return `LET $${name} = (${query})`;
  },

  createMutation(
    t: MutationEventType,
    mutationIdVar: string,
    recordIdVar: string,
    dataVar?: string
  ) {
    switch (t) {
      case 'create':
        return `CREATE ONLY $${mutationIdVar} SET mutationType = 'create', recordId = $${recordIdVar}`;
      case 'update':
        return `CREATE ONLY $${mutationIdVar} SET mutationType = 'update', recordId = $${recordIdVar}, data = $${dataVar}`;
      case 'delete':
        return `CREATE ONLY $${mutationIdVar} SET mutationType = 'delete', recordId = $${recordIdVar}`;
    }
  },

  returnObject(entries: { key: string; variable: string }[]) {
    return `RETURN {${entries.map(({ key, variable }) => `${key}: $${variable}`).join(',')}}`;
  },
};
