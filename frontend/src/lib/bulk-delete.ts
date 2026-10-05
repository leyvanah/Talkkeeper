export interface BulkDeleteResult {
  deleted: string[];
  failed: string[];
}

/**
 * Deletes the meetings one by one and says which went and which did not.
 *
 * The caller may only drop `deleted` from the list: a meeting that failed is
 * still in the archive, and hiding it would make it look deleted until the
 * next start.
 */
export async function deleteEach(
  ids: string[],
  deleteOne: (id: string) => Promise<void>,
): Promise<BulkDeleteResult> {
  const result: BulkDeleteResult = { deleted: [], failed: [] };
  for (const id of ids) {
    try {
      await deleteOne(id);
      result.deleted.push(id);
    } catch (error) {
      console.error('Failed to delete meeting', id, error);
      result.failed.push(id);
    }
  }
  return result;
}
