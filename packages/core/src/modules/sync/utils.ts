import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { parseRecordIdString } from '../../utils/index.js';

export class ArraySyncer {
  private localArray: RecordVersionArray;
  private remoteArray: RecordVersionArray;
  private needsSort = false;

  constructor(localArray: RecordVersionArray, remoteArray: RecordVersionArray) {
    this.remoteArray = remoteArray.sort((a, b) => a[0].localeCompare(b[0]));
    this.localArray = localArray.sort((a, b) => a[0].localeCompare(b[0]));
  }

  /**
   * Inserts an item into the local array
   */
  insert(recordId: string, version: number) {
    this.localArray.push([recordId, version]);
    this.needsSort = true;
  }

  /**
   * Updates the current local RecordVersionArray state.
   */
  update(recordId: string, version: number) {
    this.localArray = this.localArray.map((record) => {
      if (record[0] === recordId) {
        this.needsSort = true;
        return [recordId, version];
      }
      return record;
    });
  }

  /**
   * Deletes an item from the local array
   */
  delete(recordId: string) {
    this.localArray = this.localArray.filter((record) => record[0] !== recordId);
  }

  /**
   * Returns the difference between the local and remote arrays.
   * Includes sets of added, updated, and removed records.
   */
  nextSet(): RecordVersionDiff | null {
    if (this.needsSort) {
      this.localArray.sort((a, b) => a[0].localeCompare(b[0]));
      this.needsSort = false;
    }
    const diff = diffRecordVersionArray(this.localArray, this.remoteArray);
    return diff;
  }
}

export function diffRecordVersionArray(
  local: RecordVersionArray | null,
  remote: RecordVersionArray | null
): RecordVersionDiff {
  const localArray = local || [];
  const remoteArray = remote || [];

  // Convert arrays to Maps for O(1) lookup
  const localMap = new Map<string, number>(localArray);
  const remoteMap = new Map<string, number>(remoteArray);

  const added: string[] = [];
  const updated: string[] = [];
  const removed: string[] = [];

  // Find added and updated records
  for (const [recordId, remoteVersion] of remoteMap) {
    const localVersion = localMap.get(recordId);

    if (localVersion === undefined) {
      // Record exists in remote but not in local
      added.push(recordId);
    } else if (localVersion < remoteVersion) {
      // Record exists in both but remote has newer version
      updated.push(recordId);

      console.log('__diff__', localVersion, remoteVersion);
    }
  }

  // Find removed records
  for (const [recordId] of localMap) {
    if (!remoteMap.has(recordId)) {
      removed.push(recordId);
    }
  }

  return {
    added: added.map(parseRecordIdString),
    updated: updated.map(parseRecordIdString),
    removed: removed.map(parseRecordIdString),
  };
}
