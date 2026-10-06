/**
 * What the backend says when the built-in model's helper process has ended
 * (`HELPER_EXITED` in `summary_engine/sidecar.rs`), followed by its exit
 * status, e.g. `llama-helper exited: exit code: 0xc0000005`. Other layers may
 * put their own words in front of it.
 */
const HELPER_EXITED = 'llama-helper exited';

/**
 * The exit code of a helper that ended under a summary, or `null` when the
 * error is about something else. `''` when the helper ended without a code
 * the message shows.
 */
export function helperExitCode(errorMessage: string | null | undefined): string | null {
  if (!errorMessage) return null;
  const at = errorMessage.indexOf(HELPER_EXITED);
  if (at < 0) return null;
  const status = errorMessage.slice(at + HELPER_EXITED.length);
  return status.match(/0x[0-9a-f]+|-?\d+/i)?.[0] ?? '';
}
