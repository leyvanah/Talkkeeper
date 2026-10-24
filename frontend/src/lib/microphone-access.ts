/** Why recording from the microphone cannot start, if it cannot:
 *  `blocked` — Windows denies the app the microphone in Settings → Privacy
 *  (it is still listed then, only opening it fails); `missing` — no input
 *  device at all. */
export type MicrophoneProblem = 'blocked' | 'missing' | null;

export function microphoneProblem(inputDevices: number, blockedBySystem: boolean): MicrophoneProblem {
  if (blockedBySystem) return 'blocked';
  return inputDevices > 0 ? null : 'missing';
}

/** Which system the permission hints should talk about. */
export type PermissionHelp = 'windows' | 'macos' | 'generic';

export function permissionHelpFor(platform: string): PermissionHelp {
  if (platform === 'windows') return 'windows';
  if (platform === 'macos') return 'macos';
  return 'generic';
}

/** Message key for `base` in the wording of this system: `baseWindows`,
 *  `baseMacos` when such a variant exists, otherwise the neutral `base`. */
export function permissionKey(
  base: string,
  help: PermissionHelp,
  variants: readonly Exclude<PermissionHelp, 'generic'>[],
): string {
  if (help === 'windows' && variants.includes('windows')) return `${base}Windows`;
  if (help === 'macos' && variants.includes('macos')) return `${base}Macos`;
  return base;
}
