/** A part of the meeting page that the save button writes. */
export type SavePart = 'title' | 'summary';

export interface SaveStep {
  part: SavePart;
  /** Writes the part; fails by throwing. */
  run: () => Promise<void>;
}

/**
 * Runs every save step and returns the parts that did not make it.
 *
 * A failed step does not stop the ones after it: a title that would not save
 * is no reason to also drop the summary. Success is an empty list, and only
 * then may the page say the changes are saved.
 */
export async function runSaveSteps(steps: SaveStep[]): Promise<SavePart[]> {
  const failed: SavePart[] = [];
  for (const step of steps) {
    try {
      await step.run();
    } catch (error) {
      console.error(`Failed to save the meeting ${step.part}:`, error);
      failed.push(step.part);
    }
  }
  return failed;
}
