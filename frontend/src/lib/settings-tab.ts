/**
 * Which settings tab opens.
 *
 * The settings page unmounts whenever the archive locks — the lock screen is
 * drawn instead of the app, not over it — so a tab kept in component state was
 * lost on the way back in: the owner closed the archive from Security and came
 * back to General. The last tab is kept here instead, in memory, for this run of
 * the app only.
 */

let remembered: string | null = null

/** Notes the tab the owner is on, so the page can reopen on it. */
export function rememberSettingsTab(tab: string): void {
  remembered = tab
}

/** The tab noted last in this run, if any. */
export function rememberedSettingsTab(): string | null {
  return remembered
}

/**
 * Picks the tab to open on: one another screen asked for (a link straight to the
 * transcription models, say) wins over the remembered one, and anything that is
 * not a tab of the page falls through to `fallback`.
 */
export function pickSettingsTab(
  requested: string | null,
  previous: string | null,
  tabs: readonly string[],
  fallback: string,
): string {
  if (requested && tabs.includes(requested)) return requested
  if (previous && tabs.includes(previous)) return previous
  return fallback
}
