import { RecordId } from 'surrealdb';

export const parseRecordIdString = (id: string): RecordId<string> => {
  const [table, ...idParts] = id.split(':');
  return new RecordId(table, idParts.join(':'));
};
