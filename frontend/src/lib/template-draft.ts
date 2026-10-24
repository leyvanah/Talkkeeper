/** The new-template form, as far as clearing it is concerned. */
export interface TemplateDraft {
  name: string;
  description: string;
  sections: Array<{ title: string; instruction: string; item_format?: string }>;
}

/**
 * Whether the form holds nothing the owner typed, so "Clear" loses nothing
 * and need not ask. A section's format and empty extra sections do not count.
 */
export function isTemplateDraftEmpty(draft: TemplateDraft): boolean {
  const blank = (value: string | undefined) => !value?.trim();
  return (
    blank(draft.name) &&
    blank(draft.description) &&
    draft.sections.every((s) => blank(s.title) && blank(s.instruction) && blank(s.item_format))
  );
}
