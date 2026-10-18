/**
 * What the person page keeps while the app is open, so leaving the page for a
 * meeting and coming back loses nothing: notes typed but not yet saved, the
 * overview and the answers to questions.
 *
 * It lives in memory only. Closing the app or locking the archive forgets the
 * overview and answers (they are never written to disk); notes reach the
 * archive on their own shortly after typing stops, and a draft that failed to
 * save waits here to be saved again — also across a lock, which is exactly
 * when the save on leaving the page is refused.
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
// Bumped when everything is forgotten; answers asked before that are dropped.
let generation = 0;

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
 * dropped. Requests still running keep their spinner; what they bring back
 * describes the old data and is dropped on arrival (see `settlePersonRequest`).
 */
export function reconcilePersonAi(personId: string, signature: string): void {
  const current = personAiState(personId);
  if (current.signature === signature) return;
  states.set(personId, {
    ...EMPTY,
    signature,
    overviewPending: current.overviewPending,
    history: current.history.filter((item) => item.status === 'pending'),
  });
  notify();
}

/** Taken when a request to the model starts, handed back with its result. */
export interface PersonRequest {
  personId: string;
  generation: number;
  signature: string | null;
}

export function beginPersonRequest(
  personId: string,
  change: (state: PersonAiState) => Partial<PersonAiState>,
): PersonRequest {
  updatePersonAi(personId, change);
  return { personId, generation, signature: personAiState(personId).signature };
}

/**
 * Lands a result. After a lock it is dropped whole: the page was cleared on
 * purpose. If the person's meetings changed meanwhile, only `stale` applies —
 * it ends the spinner without showing an answer about other data.
 */
export function settlePersonRequest(
  request: PersonRequest,
  fresh: (state: PersonAiState) => Partial<PersonAiState>,
  stale: (state: PersonAiState) => Partial<PersonAiState>,
): void {
  if (request.generation !== generation) return;
  const current = personAiState(request.personId);
  updatePersonAi(request.personId, current.signature === request.signature ? fresh : stale);
}

/**
 * Forgets every overview and answer: the archive was locked, and what it
 * protects should not wait in memory for it to open again. Unsaved notes stay,
 * they are saved after unlocking.
 */
export function forgetPersonAnswers(): void {
  generation += 1;
  if (states.size === 0) return;
  states.clear();
  notify();
}

/**
 * Forgets one person: they were deleted. A draft left behind would otherwise
 * be sent to a profile that no longer exists every time the window closes.
 */
export function forgetPerson(personId: string): void {
  drafts.delete(personId);
  if (states.delete(personId)) notify();
}

/** Forgets everything; for tests. */
export function resetPersonPageMemory(): void {
  forgetPersonAnswers();
  drafts.clear();
  saving.clear();
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

// The last save started for each person.
const saving = new Map<string, Promise<void>>();

/**
 * Saves `text` after every save already started for this person, so an older
 * text can never land after a newer one — the debounced save and the one on
 * leaving the page are sent milliseconds apart. Settles like `write`.
 */
export function queueNotesSave(
  personId: string,
  text: string,
  write: (text: string) => Promise<void>,
): Promise<void> {
  const previous = saving.get(personId) ?? Promise.resolve();
  const next = previous
    .catch(() => {})
    .then(() => write(text))
    .then(() => notesSaved(personId, text));
  saving.set(personId, next);
  const forget = () => {
    if (saving.get(personId) === next) saving.delete(personId);
  };
  next.then(forget, forget);
  return next;
}

/** Every person's unsaved notes; saved when the window is about to go away. */
export function allNotesDrafts(): [string, string][] {
  return [...drafts.entries()];
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
  | 'blocked'
  /** The model settings could not be read: assume it may leave. */
  | 'unknown';

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
