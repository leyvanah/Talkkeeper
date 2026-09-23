import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslations } from 'next-intl';
import { Transcript, Summary } from '@/types';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';
import {
  detectAndCacheSummaryLanguage,
  readMeetingSummaryLanguage,
  readCachedDetectedSummaryLanguage,
} from '@/lib/summary-language-preferences';

// `t` is threaded in because this runs outside the component tree.
type Translate = (key: string) => string;

async function resolveSummaryLanguage(
  meetingId: string,
  transcriptTexts: string[],
  t: Translate
): Promise<string | null> {
  try {
    const perMeeting = await readMeetingSummaryLanguage(meetingId);
    if (perMeeting.language) return perMeeting.language;
  } catch (err) {
    console.warn('Failed to load meeting summary language:', err);
    toast.warning(t('genLangLoadFailedTitle'), {
      description: t('genLangLoadFailedDescription'),
    });
  }

  try {
    const cachedDetected = await readCachedDetectedSummaryLanguage(meetingId);
    if (cachedDetected) return cachedDetected;
  } catch (err) {
    console.warn('Failed to load cached detected summary language:', err);
  }

  try {
    const detection = await detectAndCacheSummaryLanguage(meetingId, transcriptTexts);
    if (detection.reason === 'tie') {
      toast.warning(t('genBilingualTitle'), {
        description: t('genBilingualDescription'),
      });
    }
    return detection.language;
  } catch (err) {
    console.warn('Failed to detect transcript summary language:', err);
    return null;
  }
}

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

interface UseSummaryGenerationProps {
  meeting: any;
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  isModelConfigLoading: boolean;
  selectedTemplate: string;
  onMeetingUpdated?: () => Promise<void>;
  setAiSummary: (summary: Summary | null) => void;
  onOpenModelSettings?: () => void;
}

export function useSummaryGeneration({
  meeting,
  transcripts,
  modelConfig,
  isModelConfigLoading,
  selectedTemplate,
  onMeetingUpdated,
  setAiSummary,
  onOpenModelSettings,
}: UseSummaryGenerationProps) {
  const t = useTranslations('meetingDetails');
  const [summaryStatus, setSummaryStatus] = useState<SummaryStatus>('idle');
  const [summaryError, setSummaryError] = useState<string | null>(null);

  const { startSummaryPolling, stopSummaryPolling } = useSidebar();
  const completionHandledRef = useRef(false);
  const activeMeetingIdRef = useRef(meeting.id);
  const activeProcessIdRef = useRef<string | null>(null);
  const summaryRequestGenerationRef = useRef(0);
  const setAiSummaryRef = useRef(setAiSummary);
  const onMeetingUpdatedRef = useRef(onMeetingUpdated);
  const stopSummaryPollingRef = useRef(stopSummaryPolling);
  setAiSummaryRef.current = setAiSummary;
  onMeetingUpdatedRef.current = onMeetingUpdated;
  stopSummaryPollingRef.current = stopSummaryPolling;
  activeMeetingIdRef.current = meeting.id;

  // Push path: Rust emits `summary-progress` so the UI updates without waiting
  // for the 5s poll. Polling remains as a fallback for missed events.
  useEffect(() => {
    summaryRequestGenerationRef.current += 1;
    completionHandledRef.current = false;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const registeredUnlisten = await listen<{
          meetingId: string;
          processId: string;
          stage: string;
          message?: string;
          data?: any;
        }>('summary-progress', async (event) => {
          const { meetingId, processId, stage, message, data } = event.payload || ({} as any);
          if (activeMeetingIdRef.current !== meeting.id) return;
          if (meetingId && meetingId !== meeting.id) return;
          if (!processId) return;
          // Normally only our own run is ours to render. But a run started
          // before this component mounted — the automatic pass after a
          // recording, or a run the user walked away from — has no process id
          // here to match, and its completion used to be dropped on the floor
          // while the summary sat finished in the database. The event names the
          // meeting, and this is that meeting, so it is ours to show.
          const isOurRun = processId === activeProcessIdRef.current;
          const isUnclaimedRunForThisMeeting = activeProcessIdRef.current === null;
          if (!isOurRun && !isUnclaimedRunForThisMeeting) return;

          if (stage === 'preparing' || stage === 'generating') {
            setSummaryStatus((prev) =>
              prev === 'regenerating' ? 'regenerating' : stage === 'generating' ? 'summarizing' : 'processing'
            );
            return;
          }
          if (stage === 'completed' && data) {
            if (completionHandledRef.current) return;
            completionHandledRef.current = true;
            activeProcessIdRef.current = null;
            if (data.markdown) {
              setAiSummaryRef.current({ markdown: data.markdown } as any);
            } else if (data.summary_json) {
              setAiSummaryRef.current(data as any);
            } else {
              setAiSummaryRef.current(data as any);
            }
            setSummaryStatus('completed');
            setSummaryError(null);
            stopSummaryPollingRef.current(meeting.id);
            toast.success(t('genSuccessTitle'), {
              description: message || t('genSuccessDescription'),
              duration: 4000,
            });
            await onMeetingUpdatedRef.current?.();
            return;
          }
          if (stage === 'cancelled') {
            activeProcessIdRef.current = null;
            setSummaryStatus('idle');
            stopSummaryPollingRef.current(meeting.id);
            return;
          }
          if (stage === 'error') {
            activeProcessIdRef.current = null;
            setSummaryStatus('error');
            setSummaryError(message || t('genFailedGeneric'));
            stopSummaryPollingRef.current(meeting.id);
          }
        });
        if (disposed) {
          registeredUnlisten();
        } else {
          unlisten = registeredUnlisten;
        }
      } catch (e) {
        console.warn('summary-progress listener unavailable:', e);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [meeting.id]);

  // Helper to get status message
  const getSummaryStatusMessage = useCallback((status: SummaryStatus) => {
    switch (status) {
      case 'processing':
        return t('genStatusProcessing');
      case 'summarizing':
        return t('genStatusSummarizing');
      case 'regenerating':
        return t('genStatusRegenerating');
      case 'completed':
        return t('genStatusCompleted');
      case 'error':
        return t('genStatusError');
      default:
        return '';
    }
  }, [t]);

  // Unified summary processing logic
  const processSummary = useCallback(async ({
    transcriptText,
    transcriptTexts,
    customPrompt = '',
    isRegeneration = false,
  }: {
    transcriptText: string;
    transcriptTexts?: string[];
    customPrompt?: string;
    isRegeneration?: boolean;
  }): Promise<boolean> => {
    const requestMeetingId = meeting.id;
    const requestGeneration = ++summaryRequestGenerationRef.current;
    const isCurrentRequest = () =>
      activeMeetingIdRef.current === requestMeetingId &&
      summaryRequestGenerationRef.current === requestGeneration;
    if (!isCurrentRequest()) return false;

    completionHandledRef.current = false;
    activeProcessIdRef.current = null;
    setSummaryStatus(isRegeneration ? 'regenerating' : 'processing');
    setSummaryError(null);
    let backendAccepted = false;

    try {
      if (!transcriptText.trim()) {
        throw new Error(t('genNoTranscriptText'));
      }

      console.log('Processing transcript with template:', selectedTemplate);

      // Calculate time since recording
      const timeSinceRecording = (Date.now() - new Date(meeting.created_at).getTime()) / 60000;
      if (!isCurrentRequest()) return false;

      // Track custom prompt usage if present
      if (customPrompt.trim().length > 0) {
        if (!isCurrentRequest()) return false;
      }

      // Show toast notification for generation start
      toast.info(isRegeneration ? t('genInProgressRegenerating') : t('genInProgressGenerating'), {
        description: `Using ${modelConfig.provider}/${modelConfig.model}`,
        duration: 3000,
      });

      // Resolve explicit metadata override first; Auto detects the transcript language.
      const summaryLanguage = await resolveSummaryLanguage(
        meeting.id,
        transcriptTexts?.length ? transcriptTexts : [transcriptText],
        t
      );
      if (!isCurrentRequest()) return false;

      // Process transcript and get process_id
      const result = await invokeTauri('api_process_transcript', {
        text: transcriptText,
        model: modelConfig.provider,
        modelName: modelConfig.model,
        meetingId: meeting.id,
        chunkSize: 40000,
        overlap: 1000,
        customPrompt: customPrompt,
        templateId: selectedTemplate,
        summaryLanguage,
      }) as { process_id?: unknown };

      const process_id = result.process_id;
      if (typeof process_id !== 'string' || !process_id.trim()) {
        throw new Error(t('genBackendRejected'));
      }
      backendAccepted = true;
      if (!isCurrentRequest()) return backendAccepted;
      activeProcessIdRef.current = process_id;
      console.log('Process ID:', process_id);

      // Start global polling via context
      startSummaryPolling(meeting.id, process_id, async (pollingResult) => {
        if (!isCurrentRequest()) return;
        console.log('Summary status:', pollingResult);

        // Handle cancellation
        if (pollingResult.status === 'cancelled') {
          activeProcessIdRef.current = null;
          console.log('Summary generation was cancelled');

          // Reload summary from database (backend has already restored from backup)
          try {
            const existingSummary = await invokeTauri('api_get_summary', {
              meetingId: meeting.id
            }) as any;
            if (!isCurrentRequest()) return;

            if (existingSummary?.data) {
              console.log('Restored previous summary after cancellation');
              setAiSummary(existingSummary.data);
              setSummaryStatus('completed');
            } else {
              setSummaryStatus('idle');
            }
          } catch (error) {
            if (!isCurrentRequest()) return;
            console.error('Failed to reload summary after cancellation:', error);
            setSummaryStatus('idle');
          }

          setSummaryError(null);
          return;
        }

        // Handle errors
        if (pollingResult.status === 'error' || pollingResult.status === 'failed') {
          activeProcessIdRef.current = null;
          console.error('Backend returned error:', pollingResult.error);
          const errorMessage = pollingResult.error || `Summary ${isRegeneration ? 'regeneration' : 'generation'} failed`;

          // If this was a regeneration, try to restore previous summary from database
          if (isRegeneration) {
            try {
              const existingSummary = await invokeTauri('api_get_summary', {
                meetingId: meeting.id
              }) as any;
              if (!isCurrentRequest()) return;

              if (existingSummary?.data) {
                console.log('Restored previous summary after regeneration failure');
                setAiSummary(existingSummary.data);
                setSummaryStatus('completed');
                setSummaryError(null);

                // Show error toast with restoration message
                toast.error(t('genRegenerateFailed'), {
                  description: `${errorMessage}. Your previous summary has been restored.`,
                });
                return;
              }
            } catch (error) {
              if (!isCurrentRequest()) return;
              console.error('Failed to reload summary after error:', error);
            }
          }

          // Continue with normal error handling if not regeneration or reload failed
          setSummaryError(errorMessage);
          setSummaryStatus('error');

          // Check if this is a "model is required" error
          const isModelRequiredError = errorMessage.includes('model is required') ||
            errorMessage.includes('"model":"required"') ||
            errorMessage.toLowerCase().includes('model') && errorMessage.toLowerCase().includes('required');

          // Show error toast
          toast.error(isRegeneration ? t('genRegenerateFailed') : t('genGenerateFailed'), {
            description: errorMessage.includes('Connection refused')
              ? t('genConnectionRefused')
              : errorMessage,
          });

          // Auto-open model settings modal if model is missing
          if (isModelRequiredError && onOpenModelSettings) {
            console.log('🔧 Model required error detected, opening model settings...');
            onOpenModelSettings();
          }
          return;
        }

        // Handle successful completion (also accept idle+data — some paths leave
        // the row as idle once the result is written).
        const status = (pollingResult.status || '').toLowerCase();
        if ((status === 'completed' || (status === 'idle' && pollingResult.data)) && pollingResult.data) {
          if (completionHandledRef.current) return;
          completionHandledRef.current = true;
          activeProcessIdRef.current = null;
          console.log('Summary generation completed:', pollingResult.data);

          // Check if backend returned markdown format (new flow)
          if (pollingResult.data.markdown) {
            console.log('Received markdown format from backend');
            setAiSummary({ markdown: pollingResult.data.markdown } as any);
            setSummaryStatus('completed');

            // Show success toast
            toast.success(t('genSuccessTitle'), {
              description: t('genSuccessDescription'),
              duration: 4000,
            });

            await onMeetingUpdated?.();
            return;
          }

          // Legacy format handling
          const summarySections = Object.entries(pollingResult.data).filter(([key]) => key !== 'MeetingName');
          const allEmpty = summarySections.every(([, section]) => !(section as any).blocks || (section as any).blocks.length === 0);

          if (allEmpty) {
            console.error('Summary completed but all sections empty');
            setSummaryError(t('genEmptyContent'));
            setSummaryStatus('error');
            return;
          }

          // Remove MeetingName from data before formatting
          const { MeetingName, ...summaryData } = pollingResult.data;

          // Format legacy summary data
          const formattedSummary: Summary = {};
          const sectionKeys = pollingResult.data._section_order || Object.keys(summaryData);

          for (const key of sectionKeys) {
            try {
              const section = summaryData[key];
              if (section && typeof section === 'object' && 'title' in section && 'blocks' in section) {
                const typedSection = section as { title?: string; blocks?: any[] };

                if (Array.isArray(typedSection.blocks)) {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: typedSection.blocks.map((block: any) => ({
                      ...block,
                      color: 'default',
                      content: block?.content?.trim() || ''
                    }))
                  };
                } else {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: []
                  };
                }
              }
            } catch (error) {
              console.warn(`Error processing section ${key}:`, error);
            }
          }

          setAiSummary(formattedSummary);
          setSummaryStatus('completed');

          // Show success toast
          toast.success(t('genSuccessTitle'), {
            description: t('genSuccessDescription'),
            duration: 4000,
          });

          await onMeetingUpdated?.();
        }
      });
      return true;
    } catch (error) {
      console.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary:`, error);
      const errorMessage = error instanceof Error ? error.message : t('genUnknownError');
      if (!isCurrentRequest()) return backendAccepted;
      setSummaryError(errorMessage);
      setSummaryStatus('error');
      // Note: We don't clear the summary here because the backend has already restored from backup

      toast.error(isRegeneration ? t('genRegenerateFailed') : t('genGenerateFailed'), {
        description: errorMessage,
      });
      return backendAccepted;
    }
  }, [
    meeting.id,
    meeting.created_at,
    modelConfig,
    selectedTemplate,
    startSummaryPolling,
    setAiSummary,
    onMeetingUpdated,
  ]);

  // Helper function to fetch ALL transcripts for summary generation
  const fetchAllTranscripts = useCallback(async (meetingId: string): Promise<Transcript[]> => {
    try {
      console.log('📊 Fetching all transcripts for meeting:', meetingId);

      // First, get total count by fetching first page
      const firstPage = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: 1,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      const totalCount = firstPage.total_count;
      console.log(`📊 Total transcripts in database: ${totalCount}`);

      if (totalCount === 0) {
        return [];
      }

      // Fetch all transcripts in one call
      const allData = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: totalCount,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      console.log(`✅ Fetched ${allData.transcripts.length} transcripts from database`);
      return allData.transcripts;
    } catch (error) {
      console.error('❌ Error fetching all transcripts:', error);
      toast.error(t('genFetchTranscriptsFailed'));
      return [];
    }
  }, []);

  const buildSummaryTranscriptPayload = useCallback((allTranscripts: Transcript[]) => {
    const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
      if (seconds === undefined) {
        return fallbackTimestamp;
      }
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    return {
      transcriptText: allTranscripts
        .map(t => {
          const speaker = t.speaker ? `${t.speaker}: ` : '';
          return `${formatTime(t.audio_start_time, t.timestamp)} ${speaker}${t.text}`;
        })
        .join('\n'),
      transcriptTexts: allTranscripts.map(t => t.text),
    };
  }, []);

  // Public API: Generate summary from transcripts
  const handleGenerateSummary = useCallback(async (customPrompt: string = ''): Promise<boolean> => {
    const requestMeetingId = meeting.id;
    const isCurrentMeeting = () => activeMeetingIdRef.current === requestMeetingId;
    if (!isCurrentMeeting()) return false;

    // Check if model config is still loading
    if (isModelConfigLoading) {
      console.log('⏳ Model configuration is still loading, please wait...');
      toast.info(t('genModelConfigLoading'));
      return false;
    }

    // Flip the UI into "Generating…" immediately — before the (slow) transcript
    // fetch / model checks — so auto-summary doesn't leave the empty-state
    // "Generate summary" button up for the whole prep phase.
    setSummaryStatus('processing');
    setSummaryError(null);

    // CHANGE: Fetch ALL transcripts from database, not from pagination state
    console.log('📊 Fetching all transcripts for summary generation...');
    const allTranscripts = await fetchAllTranscripts(meeting.id);
    if (!isCurrentMeeting()) return false;

    if (!allTranscripts.length) {
      const error_msg = t('genNoTranscripts');
      console.log(error_msg);
      setSummaryStatus('idle');
      toast.error(error_msg);
      return false;
    }

    console.log(`✅ Proceeding with ${allTranscripts.length} transcripts`);

    console.log('🚀 Starting summary generation with config:', {
      provider: modelConfig.provider,
      model: modelConfig.model,
      template: selectedTemplate
    });

    // Check if Ollama provider has models available
    if (modelConfig.provider === 'ollama') {
      try {
        const endpoint = modelConfig.ollamaEndpoint || null;
        const models = await invokeTauri('get_ollama_models', { endpoint }) as any[];
        if (!isCurrentMeeting()) return false;

        if (!models || models.length === 0) {
          setSummaryStatus('idle');
          toast.error(t('genOllamaNoModels'), { duration: 5000 });
          return false;
        }
      } catch (error) {
        if (!isCurrentMeeting()) return false;
        console.error('Error checking Ollama models:', error);
        const errorMessage = error instanceof Error ? error.message : String(error);
        setSummaryStatus('idle');

        if (isOllamaNotInstalledError(errorMessage)) {
          // Ollama is not installed - show specific message with download link
          toast.error(
            t('ollamaNotInstalledTitle'),
            {
              description: t('ollamaNotInstalledDescription'),
              duration: 7000,
              action: {
                label: t('ollamaDownloadAction'),
                onClick: () => invokeTauri('open_external_url', { url: 'https://ollama.com/download' })
              }
            }
          );
        } else {
          // Other error - generic message
          toast.error(t('genOllamaCheckFailed'), { duration: 5000 });
        }
        return false;
      }
    }

    // Check if built-in AI provider has models available
    if (modelConfig.provider === 'builtin-ai') {
      try {
        const selectedModel = modelConfig.model;

        if (!selectedModel) {
          setSummaryStatus('idle');
          toast.error(t('noBuiltinModelTitle'), {
            description: t('noBuiltinModelDescription'),
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return false;
        }

        // Check model readiness with filesystem refresh
        const isReady = await invokeTauri<boolean>('builtin_ai_is_model_ready', {
          modelName: selectedModel,
          refresh: true,
        });
        if (!isCurrentMeeting()) return false;

        if (!isReady) {
          // Get detailed model status
          const modelInfo = await invokeTauri<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
            modelName: selectedModel,
          });
          if (!isCurrentMeeting()) return false;

          if (modelInfo) {
            const status = modelInfo.status;

            if (status.type === 'downloading') {
              setSummaryStatus('idle');
              toast.info(t('modelDownloadingTitle'), {
                description: t('modelDownloadingDescription', { model: selectedModel, progress: status.progress }),
                duration: 5000,
              });
              return false;
            }

            if (status.type === 'not_downloaded') {
              setSummaryStatus('idle');
              toast.error(t('genBuiltinNotDownloadedTitle'), {
                description: t('genBuiltinNotDownloadedDescription', { model: selectedModel }),
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return false;
            }

            if (status.type === 'incomplete') {
              setSummaryStatus('idle');
              toast.info(t('genBuiltinIncompleteTitle'), {
                description: t('genBuiltinIncompleteDescription', { model: selectedModel }),
                duration: 7000,
              });
              onOpenModelSettings?.();
              return false;
            }

            if (status.type === 'corrupted' || status.type === 'error') {
              setSummaryStatus('idle');
              const errorDesc = status.type === 'error'
                ? status.Error || t('genBuiltinFileError')
                : t('genBuiltinFileCorrupted');
              toast.error(t('genBuiltinNotAvailableTitle'), {
                description: t('genBuiltinNotAvailableDescription', { reason: errorDesc }),
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return false;
            }
          }

          // Fallback if we couldn't get model info
          setSummaryStatus('idle');
          toast.error(t('genBuiltinNotReadyTitle'), {
            description: t('genBuiltinNotReadyDescription'),
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return false;
        }

        // Model is ready, continue to backend call
      } catch (error) {
        if (!isCurrentMeeting()) return false;
        console.error('Error validating built-in AI model:', error);
        setSummaryStatus('idle');
        toast.error(t('genBuiltinValidationFailed'), {
          description: error instanceof Error ? error.message : String(error),
          duration: 5000,
        });
        return false;
      }
    }

    const summaryPayload = buildSummaryTranscriptPayload(allTranscripts);
    if (!isCurrentMeeting()) return false;

    return processSummary({
      ...summaryPayload,
      customPrompt,
    });
  }, [meeting.id, fetchAllTranscripts, buildSummaryTranscriptPayload, processSummary, modelConfig, isModelConfigLoading, selectedTemplate]);

  // Public API: Regenerate summary from the current saved transcript
  const handleRegenerateSummary = useCallback(async (customPrompt = '') => {
    const requestMeetingId = meeting.id;
    setSummaryStatus('regenerating');
    setSummaryError(null);

    const allTranscripts = await fetchAllTranscripts(meeting.id);
    if (activeMeetingIdRef.current !== requestMeetingId) return;

    if (!allTranscripts.length) {
      console.error('No transcripts available for regeneration');
      setSummaryStatus('idle');
      toast.error(t('genNoTranscriptsForRegeneration'));
      return;
    }

    await processSummary({
      ...buildSummaryTranscriptPayload(allTranscripts),
      customPrompt,
      isRegeneration: true
    });
  }, [meeting.id, fetchAllTranscripts, buildSummaryTranscriptPayload, processSummary]);

  // Public API: Stop ongoing summary generation
  const handleStopGeneration = useCallback(async () => {
    console.log('Stopping summary generation for meeting:', meeting.id);
    summaryRequestGenerationRef.current += 1;
    activeProcessIdRef.current = null;
    completionHandledRef.current = false;

    try {
      // Call backend to cancel the summary generation
      await invokeTauri('api_cancel_summary', {
        meetingId: meeting.id
      });
      console.log('✓ Backend cancellation request sent for meeting:', meeting.id);
    } catch (error) {
      console.error('Failed to cancel summary generation:', error);
      // Continue with frontend cleanup even if backend call fails
    }

    // Stop polling
    stopSummaryPolling(meeting.id);

    // Reset status to idle
    setSummaryStatus('idle');
    setSummaryError(null);

    // Show toast notification
    toast.info(t('summaryGenerationStopped'), {
      description: t('summaryGenerationStoppedDescription'),
      duration: 3000,
    });
  }, [meeting.id, stopSummaryPolling]);

  return {
    summaryStatus,
    summaryError,
    handleGenerateSummary,
    handleRegenerateSummary,
    handleStopGeneration,
    getSummaryStatusMessage,
  };
}
