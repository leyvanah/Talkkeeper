/**
 * Whether the library can be moved to another installation as a sealed
 * package, and brought in from one.
 *
 * Off for the first release, until the package format and its import are
 * finished. Meanwhile the settings card offers only the readable export.
 *
 * The backend commands (`export_library_package`, `import_library_package`)
 * stay as they are; nothing in the interface calls them while this is off.
 * Turning transfer back on is this one line.
 */
export const LIBRARY_TRANSFER_AVAILABLE = false;

export type LibraryAction = 'readable' | 'package' | 'import';

/** What the library export card offers, in the order it shows them. */
export function libraryActions(transferAvailable: boolean = LIBRARY_TRANSFER_AVAILABLE): LibraryAction[] {
  return transferAvailable ? ['readable', 'package', 'import'] : ['readable'];
}
