/**
 * A model download refused for lack of disk space, told in the interface's
 * own words. The backend (`src-tauri/src/disk_space.rs`) marks such an error
 * with `NO_DISK_SPACE needed=<bytes> available=<bytes>`, wherever in the
 * message a command put it.
 */

export interface NoSpace {
  /** Bytes the download needs, the safety margin included. */
  needed: number;
  /** Bytes free on the disk when it was refused. */
  available: number;
}

const MARKER = /NO_DISK_SPACE needed=(\d+) available=(\d+)/;

/** The numbers of a lack-of-space refusal, or `null` for any other error. */
export function parseNoSpace(error: unknown): NoSpace | null {
  const text =
    typeof error === 'string' ? error : error instanceof Error ? error.message : String(error ?? '');
  const match = MARKER.exec(text);
  if (!match) return null;
  return { needed: Number(match[1]), available: Number(match[2]) };
}

/** A size in the interface language: "2,6 ГБ", "750 MB". */
export function formatDiskSize(bytes: number, locale: string): string {
  const gigabyte = 1024 ** 3;
  const megabyte = 1024 ** 2;
  const [value, unit] =
    bytes >= gigabyte ? [bytes / gigabyte, 'gigabyte'] : [bytes / megabyte, 'megabyte'];
  return new Intl.NumberFormat(locale, {
    style: 'unit',
    unit,
    unitDisplay: 'short',
    maximumFractionDigits: 1,
  }).format(value);
}

/**
 * The message for a lack-of-space refusal, or `null` for any other error.
 * `message` is the translated phrase with `{needed}` and `{available}`.
 */
export function noSpaceText(
  error: unknown,
  locale: string,
  message: (values: { needed: string; available: string }) => string,
): string | null {
  const noSpace = parseNoSpace(error);
  if (!noSpace) return null;
  return message({
    needed: formatDiskSize(noSpace.needed, locale),
    available: formatDiskSize(noSpace.available, locale),
  });
}
