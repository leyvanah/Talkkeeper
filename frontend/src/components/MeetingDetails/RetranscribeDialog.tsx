import React, { useState, useEffect, useRef, useMemo } from 'react';
import { useTranslations } from 'next-intl';
import { RefreshCw, Globe, Loader2, AlertCircle, CheckCircle2, X, Cpu, BookOpen } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Textarea } from '../ui/textarea';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { useRetranscription, ENHANCEMENT_STALLED } from '@/contexts/RetranscriptionContext';
import { useRouter } from 'next/navigation';
import { LANGUAGES } from '@/constants/languages';
import { useTranscriptionModels, ModelOption } from '@/hooks/useTranscriptionModels';
import Analytics from '@/lib/analytics';

interface RetranscribeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  meetingFolderPath: string | null;
  onComplete?: () => void;
}

interface WhisperVocabularyConfig {
  global: string;
  meeting: string;
}

interface PostCallTranscriptConfig {
  provider: 'live' | 'whisper' | 'parakeet';
  model: string;
}

export function RetranscribeDialog({
  open,
  onOpenChange,
  meetingId,
  meetingFolderPath,
  onComplete,
}: RetranscribeDialogProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const router = useRouter();
  const { selectedLanguage, transcriptModelConfig } = useConfig();
  const { job, start: startRetranscription, cancel: cancelRetranscription, registerJobView } =
    useRetranscription();
  // The job is the application's, not this dialog's: it keeps running while the
  // dialog is closed, and the dialog only shows it while it happens to be open.
  const isProcessing = job?.meetingId === meetingId;
  const progress = isProcessing ? job : null;
  const [error, setError] = useState<string | null>(null);
  const [selectedLang, setSelectedLang] = useState(selectedLanguage || 'auto');
  const [vocabularyTerms, setVocabularyTerms] = useState('');
  const [vocabularyScope, setVocabularyScope] = useState<'meeting' | 'global'>('meeting');
  const [savedMeetingVocabulary, setSavedMeetingVocabulary] = useState('');
  const [isClearingMeetingVocabulary, setIsClearingMeetingVocabulary] = useState(false);

  // Use centralized model fetching hook
  const {
    availableModels,
    selectedModelKey,
    setSelectedModelKey,
    loadingModels,
    hasWhisperModel,
    hasParakeetModel,
    fetchModels,
    resetSelection,
  } = useTranscriptionModels(transcriptModelConfig);

  const openWhisperSettings = () => {
    sessionStorage.setItem('meetily-settings-tab', 'Transcriptionmodels');
    sessionStorage.setItem('meetily-settings-transcription-section', 'post-call');
    onOpenChange(false);
    router.push('/settings');
  };

  // Stable refs for callbacks to avoid listener re-registration
  const onCompleteRef = useRef(onComplete);
  const onOpenChangeRef = useRef(onOpenChange);
  useEffect(() => { onCompleteRef.current = onComplete; }, [onComplete]);
  useEffect(() => { onOpenChangeRef.current = onOpenChange; }, [onOpenChange]);

  // Track previous open state to only reset on closed→open transition
  const prevOpenRef = useRef(false);

  // Helper to get selected model details (memoized)
  const selectedModelDetails = useMemo((): ModelOption | undefined => {
    if (!selectedModelKey) return undefined;
    const colonIndex = selectedModelKey.indexOf(':');
    if (colonIndex === -1) return undefined;
    const provider = selectedModelKey.slice(0, colonIndex);
    const name = selectedModelKey.slice(colonIndex + 1);
    return availableModels.find(m => m.provider === provider && m.name === name);
  }, [selectedModelKey, availableModels]);
  const isParakeetModel = selectedModelDetails?.provider === 'parakeet';

  useEffect(() => {
    if (isParakeetModel && selectedLang !== 'auto') {
      setSelectedLang('auto');
    }
  }, [isParakeetModel, selectedLang]);

  // Reset state only when dialog transitions from closed to open
  // This prevents re-initialization when config changes while dialog is already open
  useEffect(() => {
    const wasOpen = prevOpenRef.current;
    prevOpenRef.current = open;

    if (open && !wasOpen) {
      resetSelection();
      setError(null);
      setSelectedLang(selectedLanguage || 'auto');
      setVocabularyTerms('');
      setVocabularyScope('meeting');
      setSavedMeetingVocabulary('');

      // A post-call default is independent from the live model. Existing users
      // stay on the live model until they explicitly choose a post-call model.
      void invoke<PostCallTranscriptConfig>('api_get_post_call_transcript_config')
        .then((config) => fetchModels(config.provider === 'live'
          ? transcriptModelConfig
          : { provider: config.provider, model: config.model }))
        .catch((loadError) => {
          console.error('Failed to load post-call transcription config:', loadError);
          return fetchModels();
        });
      invoke<WhisperVocabularyConfig>('api_get_whisper_vocabulary', { meetingId })
        .then((config) => setSavedMeetingVocabulary(config.meeting || ''))
        .catch((loadError) => console.error('Failed to load meeting vocabulary:', loadError));
    }
  }, [open, selectedLanguage, transcriptModelConfig, fetchModels, meetingId]);

  // While this dialog is on screen it shows the job itself, so the ambient
  // indicator stands down; when it closes, the indicator takes over.
  useEffect(() => {
    if (!open || !isProcessing) return;
    return registerJobView();
  }, [open, isProcessing, registerJobView]);

  const handleStartRetranscription = async () => {
    if (!meetingFolderPath) {
      setError(t('retranscribeNoFolder'));
      return;
    }

    setError(null);

    try {
      const languageToSend = isParakeetModel ? null : selectedLang === 'auto' ? null : selectedLang;
      await Analytics.track('enhance_transcript_started', {
        language: isParakeetModel ? 'auto' : (selectedLang === 'auto' ? 'auto' : selectedLang),
        model_provider: selectedModelDetails?.provider || '',
        model_name: selectedModelDetails?.name || '',
        vocabulary_scope: vocabularyTerms.trim() ? vocabularyScope : 'unchanged'
      });

      // Settles when the job does, wherever the owner happens to be by then.
      const result = await startRetranscription({
        meetingId,
        meetingFolderPath,
        language: languageToSend,
        model: selectedModelDetails?.name || null,
        provider: selectedModelDetails?.provider || null,
        vocabularyTerms: isParakeetModel ? null : vocabularyTerms.trim() || null,
        vocabularyScope: isParakeetModel ? null : vocabularyScope,
      });

      await Analytics.track('enhance_transcript_completed', {
        success: 'true',
        duration_seconds: result.duration_seconds.toString(),
        segments_count: result.segments_count.toString(),
      });
      toast.success(t('retranscribeComplete', { count: result.segments_count }));
      onCompleteRef.current?.();
      onOpenChangeRef.current(false);
    } catch (err: any) {
      const errorMsg = err?.message === ENHANCEMENT_STALLED
        ? t('retranscribeStalled')
        : (typeof err === 'string' ? err : (err?.message || String(err)));
      setError(errorMsg);

      await Analytics.trackError('enhance_transcript_failed', errorMsg);
    }
  };

  const handleCancel = async () => {
    if (isProcessing) {
      await cancelRetranscription();
      toast.info(t('retranscribeCancelled'));
    }
    onOpenChange(false);
  };

  const clearMeetingVocabulary = async () => {
    setIsClearingMeetingVocabulary(true);
    try {
      await invoke<string>('api_save_meeting_whisper_vocabulary', {
        meetingId,
        vocabulary: '',
      });
      setSavedMeetingVocabulary('');
      toast.success(t('retranscribeVocabularyCleared'));
    } catch (clearError) {
      toast.error(typeof clearError === 'string' ? clearError : String(clearError));
    } finally {
      setIsClearingMeetingVocabulary(false);
    }
  };

  // Closing puts the work out of sight, never stops it: the job belongs to the
  // application and keeps its progress in the ambient indicator. Only Cancel
  // stops it, because only Cancel says so.
  const handleOpenChange = (newOpen: boolean) => {
    onOpenChange(newOpen);
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="max-h-[85vh] overflow-y-auto sm:max-w-[500px]"
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {isProcessing ? (
              <>
                <Loader2 className="h-5 w-5 animate-spin text-blue-600" />
                {t('retranscribeTitleProcessing')}
              </>
            ) : error ? (
              <>
                <AlertCircle className="h-5 w-5 text-red-600" />
                {t('retranscribeTitleFailed')}
              </>
            ) : (
              <>
                <RefreshCw className="h-5 w-5 text-blue-600" />
                {t('retranscribeTitle')}
              </>
            )}
          </DialogTitle>
          <DialogDescription>
            {isProcessing
              ? progress?.message || t('retranscribeDescriptionProcessing')
              : error
                ? t('retranscribeDescriptionFailed')
                : t('retranscribeDescription')}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {!isProcessing && !error && (
            !isParakeetModel ? (
              <div className="space-y-3">
                <div className="flex items-center gap-2">
                  <Globe className="h-4 w-4 text-muted-foreground" />
                  <span className="text-sm font-medium">{t('retranscribeLanguage')}</span>
                </div>
                <Select value={selectedLang} onValueChange={setSelectedLang}>
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder={t('retranscribeSelectLanguage')} />
                  </SelectTrigger>
                  <SelectContent className="max-h-60">
                    {LANGUAGES.map((lang) => (
                      <SelectItem key={lang.code} value={lang.code}>
                        {lang.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {t('retranscribeLanguageHint')}
                </p>
              </div>
            ) : (
              <div className="space-y-3">
                <div className="flex items-center gap-2">
                  <Globe className="h-4 w-4 text-muted-foreground" />
                  <span className="text-sm font-medium">{t('retranscribeLanguage')}</span>
                </div>
                <p className="text-xs text-muted-foreground">
                  {t('retranscribeParakeetLanguageHint')}
                </p>
              </div>
            )
          )}

          {!isProcessing && !error && (
            <div className="space-y-3">
              <div className="flex items-center gap-2">
                <Cpu className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">{t('retranscribeModel')}</span>
              </div>
              <Select
                value={selectedModelKey}
                onValueChange={(value) => {
                  if (value === 'install:whisper') {
                    openWhisperSettings();
                    return;
                  }
                  setSelectedModelKey(value);
                }}
                disabled={loadingModels}
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder={loadingModels ? t('retranscribeLoadingModels') : t('retranscribeSelectModel')} />
                </SelectTrigger>
                <SelectContent>
                  {availableModels.map((model) => (
                    <SelectItem key={`${model.provider}:${model.name}`} value={`${model.provider}:${model.name}`}>
                      {model.displayName} ({Math.round(model.size_mb)} MB)
                    </SelectItem>
                  ))}
                  {!hasWhisperModel && (
                    <SelectItem value="install:whisper">
                      {t('retranscribeInstallWhisperOption')}
                    </SelectItem>
                  )}
                </SelectContent>
              </Select>
              {!loadingModels && (
                <div className="grid gap-2 text-xs sm:grid-cols-2">
                  <div className="rounded-md border border-border bg-muted/30 p-3">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-medium">Parakeet</span>
                      <span className={hasParakeetModel ? 'text-emerald-600' : 'text-muted-foreground'}>
                        {hasParakeetModel ? t('retranscribeInstalled') : t('retranscribeNotInstalled')}
                      </span>
                    </div>
                    <p className="mt-1 text-muted-foreground">
                      {t('retranscribeParakeetSummary')}
                    </p>
                  </div>
                  <div className="rounded-md border border-border bg-muted/30 p-3">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-medium">Whisper</span>
                      <span className={hasWhisperModel ? 'text-emerald-600' : 'text-amber-600'}>
                        {hasWhisperModel ? t('retranscribeInstalled') : t('retranscribeOptional')}
                      </span>
                    </div>
                    <p className="mt-1 text-muted-foreground">
                      {t('retranscribeWhisperSummary')}
                    </p>
                    {!hasWhisperModel && (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="mt-2 h-7 w-full text-xs"
                        onClick={openWhisperSettings}
                      >
                        {t('retranscribeInstallWhisper')}
                      </Button>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}

          {!isProcessing && !error && !isParakeetModel && (
            <div className="space-y-3 rounded-lg border border-border p-3">
              <div className="flex items-center gap-2">
                <BookOpen className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">{t('retranscribeVocabularyTitle')}</span>
              </div>
              <Textarea
                value={vocabularyTerms}
                onChange={(event) => setVocabularyTerms(event.target.value)}
                maxLength={1000}
                rows={3}
                placeholder={t('retranscribeVocabularyPlaceholder')}
              />
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>{t('retranscribeVocabularyPriority')}</span>
                <span>{vocabularyTerms.length}/1000</span>
              </div>
              {vocabularyTerms.trim() && (
                <div className="space-y-2">
                  <span className="text-xs font-medium">{t('retranscribeVocabularyScopeLabel')}</span>
                  <Select
                    value={vocabularyScope}
                    onValueChange={(value) => setVocabularyScope(value as 'meeting' | 'global')}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="meeting">{t('retranscribeVocabularyScopeMeeting')}</SelectItem>
                      <SelectItem value="global">{t('retranscribeVocabularyScopeGlobal')}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              )}
              {savedMeetingVocabulary && (
                <div className="rounded-md bg-muted/60 px-3 py-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-xs font-medium">{t('retranscribeVocabularySaved')}</p>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-6 px-2 text-xs"
                      disabled={isClearingMeetingVocabulary}
                      onClick={clearMeetingVocabulary}
                    >
                      {isClearingMeetingVocabulary ? t('retranscribeVocabularyClearing') : tc('clear')}
                    </Button>
                  </div>
                  <p className="mt-1 max-h-16 overflow-y-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
                    {savedMeetingVocabulary}
                  </p>
                </div>
              )}
            </div>
          )}

          {isProcessing && progress && (
            <div className="space-y-2">
              <div className="relative">
                <div className="w-full bg-gray-200 rounded-full h-3">
                  <div
                    className="bg-blue-600 h-3 rounded-full transition-all duration-300 ease-out"
                    style={{ width: `${Math.min(progress.progress, 100)}%` }}
                  />
                </div>
                <div className="flex justify-between text-xs text-gray-600 mt-1">
                  <span>{progress.stage}</span>
                  <span>{Math.round(progress.progress)}%</span>
                </div>
              </div>
              <p className="text-sm text-muted-foreground text-center">
                {progress.message}
              </p>
            </div>
          )}

          {error && (
            <div className="bg-red-50 border border-red-200 rounded-lg p-3">
              <p className="text-sm text-red-800">{error}</p>
            </div>
          )}
        </div>

        <DialogFooter>
          {!isProcessing && !error && (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {tc('cancel')}
              </Button>
              <Button
                onClick={handleStartRetranscription}
                className="bg-blue-600 hover:bg-blue-700"
                disabled={!meetingFolderPath || isClearingMeetingVocabulary || loadingModels || !selectedModelDetails}
              >
                <RefreshCw className="h-4 w-4 mr-2" />
                {t('retranscribeStart')}
              </Button>
            </>
          )}
          {isProcessing && (
            <Button variant="outline" onClick={handleCancel}>
              <X className="h-4 w-4 mr-2" />
              {tc('cancel')}
            </Button>
          )}
          {error && (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                {t('retranscribeClose')}
              </Button>
              <Button
                onClick={() => {
                  setError(null);
                }}
                variant="outline"
              >
                {t('retranscribeTryAgain')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
