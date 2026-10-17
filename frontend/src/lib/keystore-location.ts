/**
 * Where the key file lives, spelled out for an owner whose copy is damaged.
 *
 * The lock screen cannot open a folder before the archive is open, so it shows
 * the full path as text: that is what the owner needs to put the saved copy back
 * by hand. The path is shown on screen only — never logged, since it carries the
 * name of the home folder.
 */

/** The key file's name inside the data folder (`KEYSTORE_FILE` in the backend). */
export const KEYSTORE_FILE = 'keystore.json'

/**
 * Joins the data folder the backend reports with the key file's name, using the
 * folder's own separator. With no folder, only the name is left to show.
 */
export function keystoreFilePath(dataDir: string | null | undefined): string {
  const folder = (dataDir ?? '').trim()
  if (!folder) return KEYSTORE_FILE
  const separator = folder.includes('\\') ? '\\' : '/'
  return `${folder.replace(/[\\/]+$/, '')}${separator}${KEYSTORE_FILE}`
}
