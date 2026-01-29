import { MutationEventType } from '../types.js';

export const surql = {
  seal(query: string) {
    return `${query};`;
  },

  tx(queries: string[]) {
    return `BEGIN TRANSACTION;${queries.join(';')};COMMIT TRANSACTION`;
  },

  create(idVar: string, dataVar: string) {
    return `CREATE ONLY $${idVar} CONTENT $${dataVar}`;
  },

  upsert(idVar: string, dataVar: string) {
    return `UPSERT ONLY $${idVar} REPLACE $${dataVar}`;
  },

  updateMerge(idVar: string, dataVar: string) {
    return `UPDATE ONLY $${idVar} MERGE $${dataVar}`;
  },

  updateSet(idVar: string, keyDataVar: ({ key: string; variable: string } | string)[]) {
    return `UPDATE $${idVar} SET ${keyDataVar
      .map((keyDataVar) =>
        typeof keyDataVar === 'string'
          ? `${keyDataVar} = $${keyDataVar}`
          : `${keyDataVar.key} = $${keyDataVar.variable}`
      )
      .join(',')}`;
  },

  delete(idVar: string) {
    return `DELETE FROM ONLY $${idVar}`;
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
