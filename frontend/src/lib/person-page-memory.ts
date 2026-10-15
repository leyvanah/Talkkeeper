/**
 * What the person page keeps while the app is open, so leaving the page for a
 * meeting and coming back loses nothing: notes typed but not yet saved, the
 * overview and the answers to questions.
 *
 * It lives in memory only. Closing the app forgets the overview and answers
 * (they are never written to disk); notes reach the archive on their own
 * shortly after typing stops, and a draft that failed to save waits here to be
 * saved again.
 */

export type AnswerStatus = 'pending' | 'complete' | 'error';

export interface PersonQuestion {
  id: number;
  question: string;
  /** The answer, or the failure text when `status` is 'error'. */
  answer: string;
  status: AnswerStatus;
  /** The model stopped on its length limit. */
  cutOff?: boolean;
}

export interface PersonAiState {
  /** The profile facts the answers were built from; see `profileSignature`. */
  signature: string | null;
  overview: string | null;
  overviewCutOff: boolean;
  overviewError: string | null;
  overviewPending: boolean;
  history: PersonQuestion[];
}

const EMPTY: PersonAiState = Object.freeze({
  signature: null,
  overview: null,
  overviewCutOff: false,
  overviewError: null,
  overviewPending: false,
  history: [],
}) as PersonAiState;

const states = new Map<string, PersonAiState>();
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

/** Same object until something changes, as `useSyncExternalStore` needs. */
export function personAiState(personId: string): PersonAiState {
  return states.get(personId) ?? EMPTY;
}

export function subscribePersonAi(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Changes one person's state. Requests that finish after the page was left
 * land here too, so the answer is waiting when the owner comes back.
 */
export function updatePersonAi(
  personId: string,
  change: (state: PersonAiState) => Partial<PersonAiState>,
): void {
  const current = personAiState(personId);
  states.set(personId, { ...current, ...change(current) });
  notify();
}

/**
 * The facts that, when they change, make an earlier overview describe a
 * different set of meetings than the one now on the page.
 */
export function profileSignature(profile: {
  displayName: string;
  meetingCount: number;
  messageCount: number;
  lastSeenAt?: string | null;
}): string {
  return JSON.stringify([
    profile.displayName,
    profile.meetingCount,
    profile.messageCount,
    profile.lastSeenAt ?? null,
  ]);
}

/**
 * Called with the freshly loaded profile: answers built from other data are
 * dropped. One still being written is kept, it will arrive for this person.
 */
export function reconcilePersonAi(personId: string, signature: string): void {
  const current = personAiState(personId);
  if (current.signature === signature) return;
  const stillWriting = current.overviewPending || current.history.some((item) => item.status === 'pending');
  if (current.signature === null || stillWriting) {
    states.set(personId, { ...current, signature });
  } else {
    states.set(personId, { ...EMPTY, signature });
  }
  notify();
}

/** Forgets everything; for tests. */
export function resetPersonPageMemory(): void {
  states.clear();
  drafts.clear();
  notify();
}

// Notes typed but not confirmed saved, by person.
const drafts = new Map<string, string>();

export function notesDraft(personId: string): string | undefined {
  return drafts.get(personId);
}

export function keepNotesDraft(personId: string, text: string): void {
  drafts.set(personId, text);
}

/**
 * Drops the draft once `saved` reached the archive — unless the owner kept
 * typing meanwhile, and then the newer text still waits for its own save.
 */
export function notesSaved(personId: string, saved: string): void {
  if (drafts.get(personId) === saved) drafts.delete(personId);
}

/** Where the person page's questions go with the chosen model. */
export type AnswerDestination =
  /** No model is chosen yet. */
  | 'none'
  /** Built-in model, or Ollama / own server on this computer. */
  | 'here'
  /** A cloud provider: Claude, Groq, OpenAI, OpenRouter. */
  | 'cloud'
  /** Ollama or an own server at an address on another machine. */
  | 'remote'
  /** Would leave this computer, but "This computer only" refuses it. */
  | 'blocked';

export function answerDestination(options: {
  hasModel: boolean;
  leavesThisComputer: boolean;
  isCloud: boolean;
  localOnly: boolean;
}): AnswerDestination {
  if (!options.hasModel) return 'none';
  if (!options.leavesThisComputer) return 'here';
  if (options.localOnly) return 'blocked';
  return options.isCloud ? 'cloud' : 'remote';
}

const PROVIDER_NAMES: Record<string, string> = {
  claude: 'Claude',
  groq: 'Groq',
  openai: 'OpenAI',
  openrouter: 'OpenRouter',
  ollama: 'Ollama',
};

/** The name the owner chose the provider by in settings; the model id as is. */
export function providerName(provider: string): string {
  return PROVIDER_NAMES[provider] ?? provider;
}
