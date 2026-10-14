/**
 * Ctrl+C and Ctrl+X on a selection, routed through `copyText`.
 *
 * The copy buttons already write privately, but selecting text in a summary or
 * a transcript and pressing Ctrl+C went the browser's way: the text (as HTML)
 * landed in clipboard history (Win+V) like any other copy. This listens to
 * `copy` and `cut` on the whole window in the capture phase, before the summary
 * editor's own handler, and writes the selected text privately instead.
 * Formatting is lost on paste: plain text only.
 */

/** The part of a text field that matters here. */
type TextField = {
  type: string
  value: string
  selectionStart: number | null
  selectionEnd: number | null
  readOnly?: boolean
  disabled?: boolean
}

type ClipboardLikeEvent = {
  type: string
  preventDefault(): void
  stopImmediatePropagation(): void
}

export type CopyEnvironment = {
  /** The focused element, if it is an `<input>` or `<textarea>`. */
  focusedField(): TextField | null
  /**
   * The selected text outside text fields; `null` when nothing is selected.
   * A selection may hold no text (an image block) and still be a selection.
   */
  selectedText(): string | null
  /** Whether the selection outside text fields can be edited (contenteditable). */
  selectionIsEditable(): boolean
  /** Removes the selection the way the Delete key would. */
  deleteSelection(): void
}

/**
 * The text a copy would take: the selection in a focused field, or on the page;
 * `null` when there is nothing to take and the browser can be left alone.
 *
 * A password field gives nothing — the browser never copies one, and neither
 * does this. Nor does a field without a selection API (`type="number"`): its
 * selection cannot be read, and such fields hold settings, not archive text.
 */
export function selectionToCopy(env: CopyEnvironment): string | null {
  const field = env.focusedField()
  if (field) {
    if (field.type === 'password') return null
    const { selectionStart: start, selectionEnd: end } = field
    if (start === null || end === null || start === end) return null
    return field.value.slice(start, end)
  }
  return env.selectedText()
}

function canDelete(env: CopyEnvironment): boolean {
  const field = env.focusedField()
  if (field) return !field.readOnly && !field.disabled
  return env.selectionIsEditable()
}

/**
 * Handles one `copy` or `cut` event. Nothing selected — the event is left
 * alone. Otherwise the browser's own copy and every later handler are stopped,
 * so nothing selected ever reaches the clipboard the ordinary way. A copy, or a
 * cut of text that can be edited, goes to `copy`, and the cut then removes the
 * selection; a cut of text that cannot be edited does nothing, as before.
 */
export function handleClipboardEvent(
  event: ClipboardLikeEvent,
  env: CopyEnvironment,
  copy: (text: string) => Promise<void>,
  onError: (error: unknown) => void,
): void {
  const text = selectionToCopy(env)
  if (text === null) return

  event.preventDefault()
  event.stopImmediatePropagation()

  const cut = event.type === 'cut'
  if (cut && !canDelete(env)) return
  if (text) copy(text).catch(onError)
  if (cut) env.deleteSelection()
}

function browserEnvironment(doc: Document): CopyEnvironment {
  return {
    focusedField() {
      const el = doc.activeElement
      if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) return el
      return null
    },
    selectedText() {
      const selection = doc.getSelection()
      if (!selection || selection.isCollapsed) return null
      return selection.toString()
    },
    selectionIsEditable() {
      // Both ends: a selection running out of the editor cannot be deleted.
      const selection = doc.getSelection()
      const editable = (node: Node | null | undefined) => {
        const el = node instanceof HTMLElement ? node : node?.parentElement
        return el?.isContentEditable ?? false
      }
      return editable(selection?.anchorNode) && editable(selection?.focusNode)
    },
    deleteSelection() {
      // Goes through the editor's input handling like a key press, so the
      // summary editor and React-controlled fields keep their state in step.
      doc.execCommand('delete')
    },
  }
}

/** Starts routing Ctrl+C / Ctrl+X through `copy`; returns the way to stop. */
export function installPrivateCopy(
  win: Window,
  copy: (text: string) => Promise<void>,
  onError: (error: unknown) => void,
): () => void {
  const env = browserEnvironment(win.document)
  const listener = (event: Event) => handleClipboardEvent(event, env, copy, onError)
  win.addEventListener('copy', listener, true)
  win.addEventListener('cut', listener, true)
  return () => {
    win.removeEventListener('copy', listener, true)
    win.removeEventListener('cut', listener, true)
  }
}
