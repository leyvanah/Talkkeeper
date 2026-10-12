interface OnboardingSummaryModelStatusInput {
  selectedModel: string;
  recommendedModel: string;
  selectedModelReady: boolean;
}

interface OnboardingSummaryModelStatus {
  selectedSummaryModel: string;
  summaryModelDownloaded: boolean;
}

const SUMMARY_MODEL_SIZES_MB: Record<string, number> = {
  'qwen3.5:2b': 1221,
  'qwen3.5:4b': 2614,
  'gemma3:1b': 1019,
  'gemma3:4b': 2374,
};

/** Download sizes of the speech recognition engines onboarding offers. */
export const STT_DOWNLOAD_MB = { parakeet: 670, gigaam: 215 } as const;

/**
 * What onboarding is about to download, in MB: the recognition engine and
 * the summary model, each only if it is not on disk already.
 */
export function onboardingDownloadMb({
  sttEngine,
  sttDownloaded,
  summaryModel,
  summaryDownloaded,
}: {
  sttEngine: keyof typeof STT_DOWNLOAD_MB;
  sttDownloaded: boolean;
  summaryModel: string;
  summaryDownloaded: boolean;
}): number {
  return (
    (sttDownloaded ? 0 : STT_DOWNLOAD_MB[sttEngine]) +
    (summaryDownloaded ? 0 : getSummaryModelSizeMb(summaryModel))
  );
}

export function resolveOnboardingSummaryModelStatus({
  selectedModel,
  recommendedModel,
  selectedModelReady,
}: OnboardingSummaryModelStatusInput): OnboardingSummaryModelStatus {
  const selectedSummaryModel = selectedModel || recommendedModel;

  return {
    selectedSummaryModel,
    summaryModelDownloaded: Boolean(selectedSummaryModel && selectedModelReady),
  };
}

export function getSummaryModelSizeMb(model: string): number {
  return SUMMARY_MODEL_SIZES_MB[model] ?? 0;
}

export function getDownloadTotalMb(totalMb: number | null | undefined, model: string): number {
  return totalMb || getSummaryModelSizeMb(model);
}

export function formatSummaryModelSizeLabelFromMb(sizeMb: number): string {
  if (sizeMb === 0) {
    return '';
  }

  if (sizeMb >= 1024) {
    return `~${(sizeMb / 1024).toFixed(1)} GiB`;
  }

  return `~${sizeMb} MiB`;
}

export function getSummaryModelSizeLabel(model: string): string {
  return formatSummaryModelSizeLabelFromMb(getSummaryModelSizeMb(model));
}
