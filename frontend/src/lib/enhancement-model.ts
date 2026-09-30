export type EnhancementProvider = 'whisper' | 'parakeet' | 'gigaam' | 'externalStt';

export interface ModelChoice {
  provider: EnhancementProvider;
  name: string;
}

export interface RequestedModel {
  provider?: string;
  model?: string;
}

export interface EnhancementChoice {
  model: ModelChoice;
  /** The requested kind of model that was not available and was stepped over. */
  replaced: EnhancementProvider | null;
}

export const PROVIDER_LABELS: Record<EnhancementProvider, string> = {
  whisper: 'Whisper',
  parakeet: 'Parakeet',
  gigaam: 'GigaAM',
  externalStt: 'External STT',
};

const providerOf = (requested: RequestedModel | undefined) =>
  requested?.provider === 'localWhisper' ? 'whisper' : requested?.provider;

/** The exact model, or another one of the same kind. */
function find(requested: RequestedModel | undefined, pool: ModelChoice[]): ModelChoice | undefined {
  const provider = providerOf(requested);
  return pool.find((model) => model.provider === provider && model.name === requested?.model)
    ?? pool.find((model) => model.provider === provider);
}

/**
 * The post-call choice when its model is on disk, then the live model, then
 * any local model. A missing model must not cost the owner the enhancement:
 * it used to fail outright when the chosen model had been removed. Only the
 * owner's own choice may send audio to the external service — a fallback never
 * does.
 */
export function chooseEnhancementModel(
  available: ModelChoice[],
  postCall: RequestedModel,
  live: RequestedModel | undefined,
): EnhancementChoice | null {
  const followsLive = !postCall.provider || postCall.provider === 'live';
  const requested = followsLive ? live : postCall;
  const chosen = find(requested, available);
  if (chosen) return { model: chosen, replaced: null };

  const local = available.filter((model) => model.provider !== 'externalStt');
  const wanted = providerOf(requested);
  const replaced = wanted && wanted in PROVIDER_LABELS ? wanted as EnhancementProvider : null;
  // Parakeet before GigaAM: it is multilingual, GigaAM knows only Russian.
  const fallback = (followsLive ? undefined : find(live, local))
    ?? local.find((model) => model.provider === 'parakeet')
    ?? local[0];
  return fallback ? { model: fallback, replaced } : null;
}
