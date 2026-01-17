import { RecordVersionArray, RecordVersionDiff } from '../../types.js';
import { parseRecordIdString } from '../../utils/index.js';

export class ArraySyncer {
  private localArray: RecordVersionArray;
  private remoteArray: RecordVersionArray;

  constructor(localArray: RecordVersionArray, remoteArray: RecordVersionArray) {
    this.remoteArray = remoteArray || [];
    this.localArray = localArray || [];
  }

  /**
   * Updates the current local RecordVersionArray state.
   */
  update(local: RecordVersionArray) {
    this.localArray = local || [];
  }

  /**
   * Returns the difference between the local and remote arrays.
   * Includes sets of added, updated, and removed records.
   */
  nextSet(): RecordVersionDiff {
    return diffRecordVersionArray(this.localArray, this.remoteArray);
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
