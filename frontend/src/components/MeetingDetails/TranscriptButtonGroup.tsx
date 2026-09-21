"use client";

import { useState, useCallback, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Download, FolderOpen, RefreshCw, Users, Loader2 } from 'lucide-react';
import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenExport?: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


/** One way of telling the speakers apart, as a choice in the speakers dialog. */
function SpeakerMethodOption({
  selected,
  onSelect,
  title,
  hint,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  hint: string;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      // The theme recolours border utilities, so the chosen one is set here.
      style={{ borderColor: selected ? 'var(--af-accent)' : 'var(--af-border)' }}
      className={`flex w-full items-start gap-2.5 rounded-md border px-3 py-2 text-left transition-colors ${
        selected ? 'bg-[var(--af-hover)]' : 'hover:bg-[var(--af-hover)]'
      }`}
    >
      <span
        aria-hidden
        className="mt-1 flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border"
        style={{ borderColor: selected ? 'var(--af-accent)' : 'var(--af-text-3)' }}
      >
        {selected && <span className="h-1.5 w-1.5 rounded-full bg-[var(--af-accent)]" />}
      </span>
      <span>
        <span className="block text-sm font-medium text-[var(--af-text,#111827)]">{title}</span>
        <span className="mt-0.5 block text-xs text-gray-500">{hint}</span>
      </span>
    </button>
  );
}

export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenExport,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  // Speaker diarization ("who spoke when") — only offered when the local
  // models are installed.
  const [diarizeAvailable, setDiarizeAvailable] = useState(false);
  const [isDiarizing, setIsDiarizing] = useState(false);
  const [showSpeakerDialog, setShowSpeakerDialog] = useState(false);
  const [expectedSpeakers, setExpectedSpeakers] = useState<string>('');

  useEffect(() => {
    invoke<boolean>('diarization_models_available')
      .then(setDiarizeAvailable)
      .catch(() => setDiarizeAvailable(false));
  }, []);

  // A recording with a track per side is labelled by device: the microphone
  // is the owner, the speakers the other side. No model, no count to ask for -
  // the model is only the choice when several people shared the far side.
  const [hasDeviceTracks, setHasDeviceTracks] = useState(false);
  const [byModel, setByModel] = useState(false);
  useEffect(() => {
    if (!meetingId) {
      setHasDeviceTracks(false);
      return;
    }
    let cancelled = false;
    invoke<boolean>('diarization_has_device_tracks', { meetingId })
      .then((value) => !cancelled && setHasDeviceTracks(value))
      .catch(() => !cancelled && setHasDeviceTracks(false));
    return () => {
      cancelled = true;
    };
  }, [meetingId]);
  const canIdentifySpeakers = diarizeAvailable || hasDeviceTracks;
  const byDevice = hasDeviceTracks && !byModel;

  const handleRetranscribeComplete = useCallback(async () => {
    // Retranscription replaces transcript rows and therefore clears speaker
    // labels. Immediately re-run the improved offline pass (dual tracks when
    // available; enrolled voiceprint fallback for older mixed recordings).
    if (meetingId && canIdentifySpeakers) {
      const toastId = toast.loading(t('refreshingSpeakersTitle'), {
        description: t('refreshingSpeakersDescription'),
      });
      try {
        const knownCount = parseInt(expectedSpeakers, 10);
        await invoke('diarize_meeting', {
          meetingId,
          numSpeakers: Number.isFinite(knownCount) && knownCount > 0 ? knownCount : null,
        });
        toast.success(t('speakersRefreshed'), { id: toastId });
      } catch (error) {
        console.warn('Post-retranscription diarization skipped:', error);
        toast.warning(t('speakerRefreshUnavailable'), { id: toastId });
      }
    }

    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [meetingId, canIdentifySpeakers, expectedSpeakers, onRefetchTranscripts, t]);

  const handleIdentifySpeakers = useCallback(async (expected?: number, method?: 'device' | 'model') => {
    if (!meetingId || isDiarizing) return;
    setShowSpeakerDialog(false);
    setIsDiarizing(true);
    const toastId = toast.loading(t('identifyingSpeakersTitle'), {
      description: t('identifyingSpeakersDescription'),
    });
    try {
      const res = await invoke<{ num_speakers: number; labeled: number }>('diarize_meeting', {
        meetingId,
        numSpeakers: expected ?? null,
        method: method ?? null,
      });
      toast.success(
        res.num_speakers > 0
          ? t('speakersFound', { count: res.num_speakers })
          : t('noSpeakersDetected'),
        { id: toastId, description: t('segmentsLabeled', { count: res.labeled }) }
      );
      if (onRefetchTranscripts) {
        await onRefetchTranscripts();
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      toast.error(t('speakerIdentificationFailed'), { id: toastId, description: msg });
    } finally {
      setIsDiarizing(false);
    }
  }, [meetingId, isDiarizing, onRefetchTranscripts, t]);

  return (
    <div className="flex w-max min-w-full shrink-0 items-center justify-end">
      <ButtonGroup className="shrink-0">
        <Button
          variant="outline"
          size="sm"
          className="transcript-action-button h-9 w-9 shrink-0 px-0"
          onClick={() => {
            onCopyTranscript();
          }}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? t('noTranscriptAvailable') : t('copyTranscript')}
        >
          <Copy size={16} />
          <span className="transcript-action-label">{t('copy')}</span>
        </Button>

        {onOpenExport && (
          <Button
            variant="outline"
            size="sm"
            className="transcript-action-button h-9 w-9 shrink-0 px-0"
            onClick={() => {
              onOpenExport();
            }}
            disabled={transcriptCount === 0}
            title={transcriptCount === 0 ? t('noMeetingContent') : t('exportMeeting')}
          >
            <Download size={16} />
            <span className="transcript-action-label">{t('export')}</span>
          </Button>
        )}

        <Button
          size="sm"
          variant="outline"
          className="transcript-action-button h-9 w-9 shrink-0 px-0"
          onClick={() => {
            onOpenMeetingFolder();
          }}
          title={t('openRecordingFolder')}
        >
          <FolderOpen size={16} />
          <span className="transcript-action-label">{t('recordingFolder')}</span>
        </Button>

        {canIdentifySpeakers && meetingId && (
          <Button
            size="sm"
            variant="outline"
            className="transcript-action-button h-9 w-9 shrink-0 px-0"
            onClick={() => {
              setExpectedSpeakers('');
              setByModel(false);
              setShowSpeakerDialog(true);
            }}
            disabled={isDiarizing || transcriptCount === 0}
            title={transcriptCount === 0 ? t('noTranscriptAvailable') : t('identifySpeakersTooltip')}
          >
            {isDiarizing ? (
              <Loader2 className="animate-spin" size={16} />
            ) : (
              <Users size={16} />
            )}
            <span className="transcript-action-label">{isDiarizing ? t('speakersWorking') : t('speakers')}</span>
          </Button>
        )}

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="transcript-action-button h-9 w-9 shrink-0 border-blue-500/30 bg-blue-500/10 px-0 text-blue-300 hover:bg-blue-500/20"
            onClick={() => {
              setShowRetranscribeDialog(true);
            }}
            title={t('enhanceTooltip')}
          >
            <RefreshCw size={16} />
            <span className="transcript-action-label">{t('enhance')}</span>
          </Button>
        )}
      </ButtonGroup>

      {/* Ask how many speakers to expect before diarizing */}
      <Dialog open={showSpeakerDialog} onOpenChange={setShowSpeakerDialog}>
        <DialogContent aria-describedby={undefined} className="sm:max-w-md">
          <DialogTitle className="flex items-center gap-2 text-base">
            <Users size={18} className="text-blue-500" />
            {t('identifySpeakersDialogTitle')}
          </DialogTitle>
          <div className="mt-2 space-y-3">
            {hasDeviceTracks && (
              <div role="radiogroup" aria-label={t('identifySpeakersDialogTitle')} className="space-y-2">
                <SpeakerMethodOption
                  selected={!byModel}
                  onSelect={() => setByModel(false)}
                  title={t('speakersByDevice')}
                  hint={t('speakersByDeviceHint')}
                />
                {diarizeAvailable && (
                  <SpeakerMethodOption
                    selected={byModel}
                    onSelect={() => setByModel(true)}
                    title={t('speakersByModel')}
                    hint={t('speakersByModelHint')}
                  />
                )}
              </div>
            )}
            {!byDevice && (
            <>
            <p className="text-sm text-gray-500">
              {t.rich('identifySpeakersDialogHint', {
                b: (chunks) => (
                  <span className="text-[var(--af-text,#374151)] font-medium">{chunks}</span>
                ),
              })}
            </p>
            <input
              type="number"
              min={1}
              max={20}
              autoFocus={!hasDeviceTracks}
              value={expectedSpeakers}
              onChange={(e) => setExpectedSpeakers(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  const n = parseInt(expectedSpeakers, 10);
                  handleIdentifySpeakers(Number.isFinite(n) && n > 0 ? n : undefined, 'model');
                }
              }}
              placeholder={t('autoDetect')}
              className="w-full rounded-md border border-[var(--af-border,#d1d5db)] bg-[var(--af-panel-2,#fff)] px-3 py-2 text-sm text-[var(--af-text,#111827)] outline-none focus:ring-2 focus:ring-blue-500"
            />
            <div className="flex flex-wrap gap-1.5">
              {[2, 3, 4, 5, 6, 8].map((n) => (
                <button
                  key={n}
                  type="button"
                  onClick={() => setExpectedSpeakers(String(n))}
                  className={`rounded-full border px-3 py-1 text-xs transition-colors ${
                    expectedSpeakers === String(n)
                      ? 'border-blue-500 bg-blue-50 text-blue-600'
                      : 'border-[var(--af-border,#e5e7eb)] text-gray-500 hover:border-blue-400 hover:text-blue-500'
                  }`}
                >
                  {n}
                </button>
              ))}
            </div>
            </>
            )}
          </div>
          <div className="mt-4 flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setShowSpeakerDialog(false)}>
              {tc('cancel')}
            </Button>
            <Button
              size="sm"
              className="bg-blue-600 text-white hover:bg-blue-700"
              onClick={() => {
                if (byDevice) {
                  handleIdentifySpeakers(undefined, 'device');
                  return;
                }
                const n = parseInt(expectedSpeakers, 10);
                handleIdentifySpeakers(Number.isFinite(n) && n > 0 ? n : undefined, 'model');
              }}
            >
              <Users size={16} className="mr-1.5" />
              {byDevice
                ? t('speakersLabelByDevice')
                : expectedSpeakers
                  ? t('findSpeakersCount', { count: expectedSpeakers })
                  : t('autoDetect')}
            </Button>
          </div>
        </DialogContent>
      </Dialog>

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
