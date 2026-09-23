"use client";

/**
 * Right-hand panel on the meeting-details screen, in three tabs:
 *   - Summary: generation/regeneration (SummaryGenerator/Updater button
 *     groups + language picker) and the read view via <InsightTabs> (AI
 *     Summary / Action Items / Key Topics);
 *   - Notes: the owner's own notes — a placeholder until they exist;
 *   - Chat: questions about the meeting, via <MeetingChat>.
 * The chosen tab is remembered per machine.
 *
 * Width is intentionally wide (`w-[62%] max-w-[960px] min-w-[520px]`) so the
 * transcript column (middle) and this panel share the 3-column details layout.
 *
 * Note: this is the details-screen panel. There is a *separate* live-screen
 * TranscriptPanel under app/_components/ — do not confuse the two (see CLAUDE.md).
 */

import { Summary, SummaryResponse, Transcript } from '@/types';
import { EditableTitle } from '@/components/EditableTitle';
import { BlockNoteSummaryView, BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { EmptyStateSummary } from '@/components/EmptyStateSummary';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { SummaryGeneratorButtonGroup, SummaryLanguageChoice } from './SummaryGeneratorButtonGroup';
import { SummaryUpdaterButtonGroup } from './SummaryUpdaterButtonGroup';
import { InsightTabs } from './InsightTabs';
import { MeetingChat } from './MeetingChat';
import { NotebookPen } from 'lucide-react';
import { useCallback, useEffect, useRef, useState, RefObject } from 'react';
import { useTranslations } from 'next-intl';
import { toast } from 'sonner';
import { LanguagePickerPopover } from '@/components/LanguagePickerPopover';
import { useRecentLanguages } from '@/hooks/useRecentLanguages';
import { labelForCode } from '@/lib/summary-languages';
import {
  readMeetingSummaryLanguage,
  saveMeetingSummaryLanguage,
  SummaryLanguageStorage,
} from '@/lib/summary-language-preferences';

type SideTab = 'summary' | 'notes' | 'chat';

/** A reading preference, kept per machine rather than per meeting. */
const TAB_STORAGE_KEY = 'meeting_side_tab';

interface SummaryPanelProps {
  meeting: {
    id: string;
    title: string;
    created_at: string;
  };
  meetingTitle: string;
  onTitleChange: (title: string) => void;
  isEditingTitle: boolean;
  onStartEditTitle: () => void;
  onFinishEditTitle: () => void;
  isTitleDirty: boolean;
  summaryRef: RefObject<BlockNoteSummaryViewRef>;
  isSaving: boolean;
  onSaveAll: () => Promise<void>;
  onCopySummary: () => Promise<void>;
  onCopyTranscript?: () => Promise<void>;
  onOpenExport?: () => void;
  onOpenFolder: () => Promise<void>;
  aiSummary: Summary | null;
  summaryStatus: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  onGenerateSummary: (customPrompt: string) => Promise<boolean>;
  onStopGeneration: () => void;
  customPrompt: string;
  summaryResponse: SummaryResponse | null;
  onSaveSummary: (summary: Summary | { markdown?: string; summary_json?: any[] }) => Promise<void>;
  onSummaryChange: (summary: Summary) => void;
  onDirtyChange: (isDirty: boolean) => void;
  summaryError: string | null;
  onRequestRegenerate: () => void;
  getSummaryStatusMessage: (status: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error') => string;
  availableTemplates: Array<{ id: string, name: string, description: string }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
  onManageTemplates?: () => void;
  isModelConfigLoading?: boolean;
  onOpenModelSettings?: (openFn: () => void) => void;
}

export function SummaryPanel({
  meeting,
  meetingTitle,
  onTitleChange,
  isEditingTitle,
  onStartEditTitle,
  onFinishEditTitle,
  isTitleDirty,
  summaryRef,
  isSaving,
  onSaveAll,
  onCopySummary,
  onCopyTranscript,
  onOpenExport,
  onOpenFolder,
  aiSummary,
  summaryStatus,
  transcripts,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onGenerateSummary,
  onStopGeneration,
  customPrompt,
  summaryResponse,
  onSaveSummary,
  onSummaryChange,
  onDirtyChange,
  summaryError,
  onRequestRegenerate,
  getSummaryStatusMessage,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
  onManageTemplates,
  isModelConfigLoading = false,
  onOpenModelSettings
}: SummaryPanelProps) {
  const t = useTranslations('meetingDetails');
  const [tab, setTab] = useState<SideTab>('summary');
  useEffect(() => {
    try {
      const stored = localStorage.getItem(TAB_STORAGE_KEY);
      if (stored === 'notes' || stored === 'chat') setTab(stored);
    } catch {
      // Storage can be unavailable; the summary is the default anyway.
    }
  }, []);
  const chooseTab = useCallback((next: SideTab) => {
    setTab(next);
    try {
      localStorage.setItem(TAB_STORAGE_KEY, next);
    } catch {
      // Not remembered this time; nothing else depends on it.
    }
  }, []);
  const [summaryLang, setSummaryLang] = useState<string | null>(null);
  const [summaryLangStorage, setSummaryLangStorage] = useState<SummaryLanguageStorage>('metadata');
  const languageLoadVersionRef = useRef(0);
  const activeMeetingIdRef = useRef(meeting.id);
  const languageSaveVersionRef = useRef(0);
  const languageSaveLoopRunningRef = useRef(false);
  const latestLanguageSaveRequestRef = useRef<{
    version: number;
    meetingId: string;
    language: string | null;
    rollback: {
      language: string | null;
      storage: SummaryLanguageStorage;
    };
  } | null>(null);
  activeMeetingIdRef.current = meeting.id;
  const { addRecent } = useRecentLanguages();

  const effectiveLangLabel = summaryLang ? labelForCode(summaryLang) : t('summaryLangAuto');
  const isLocalFallbackLanguage = summaryLangStorage === 'local_fallback';
  const autoSubtitle = isLocalFallbackLanguage
    ? t('summaryLangLocalFallbackSubtitle')
    : t('summaryLangAutoSubtitle');

  useEffect(() => {
    let cancelled = false;
    const loadVersion = languageLoadVersionRef.current + 1;
    languageLoadVersionRef.current = loadVersion;

    const loadSummaryLanguage = async () => {
      try {
        const stored = await readMeetingSummaryLanguage(meeting.id);
        if (!cancelled && languageLoadVersionRef.current === loadVersion) {
          setSummaryLang(stored.language);
          setSummaryLangStorage(stored.storage);
        }
      } catch (err) {
        console.error('Failed to load summary language:', err);
        toast.warning(t('summaryLangLoadFailedTitle'), {
          description: t('summaryLangLoadFailedDescription'),
        });
        if (!cancelled && languageLoadVersionRef.current === loadVersion) setSummaryLang(null);
      }
    };

    loadSummaryLanguage();

    return () => {
      cancelled = true;
    };
  }, [meeting.id]);

  const persistLatestLanguageSelection = async () => {
    if (languageSaveLoopRunningRef.current) return;
    languageSaveLoopRunningRef.current = true;

    try {
      while (true) {
        const request = latestLanguageSaveRequestRef.current;
        if (!request) return;

        try {
          const saved = await saveMeetingSummaryLanguage(request.meetingId, request.language);
          const latest = latestLanguageSaveRequestRef.current;
          if (
            latest?.version === request.version &&
            activeMeetingIdRef.current === request.meetingId
          ) {
            setSummaryLang(saved.language);
            setSummaryLangStorage(saved.storage);
            if (saved.storage === 'local_fallback') {
              toast.info(t('summaryLangSavedLocallyTitle'), {
                description: t('summaryLangSavedLocallyDescription'),
              });
            }
            if (request.language) {
              addRecent(request.language);
            }
            return;
          }

          if (latest?.version === request.version) return;
        } catch (err) {
          const latest = latestLanguageSaveRequestRef.current;
          if (
            latest?.version === request.version &&
            activeMeetingIdRef.current === request.meetingId
          ) {
            console.error('Failed to persist summary language:', err);
            toast.error(t('summaryLangSaveFailed'));
            setSummaryLang(request.rollback.language);
            setSummaryLangStorage(request.rollback.storage);
            return;
          }

          console.warn('Ignoring failed stale summary language save:', err);
          if (latest?.version === request.version) return;
        }
      }
    } finally {
      languageSaveLoopRunningRef.current = false;
    }
  };

  const handleLangChange = (code: string | null) => {
    const previous = summaryLang;
    const previousStorage = summaryLangStorage;
    const nextStored = code;
    languageLoadVersionRef.current += 1;
    latestLanguageSaveRequestRef.current = {
      version: languageSaveVersionRef.current + 1,
      meetingId: meeting.id,
      language: nextStored,
      rollback: {
        language: previous,
        storage: previousStorage,
      },
    };
    languageSaveVersionRef.current += 1;
    setSummaryLang(nextStored);
    void persistLatestLanguageSelection();
  };

  const isSummaryLoading = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';

  const language: SummaryLanguageChoice = {
    label: effectiveLangLabel,
    picker: (close) => (
      <LanguagePickerPopover
        value={summaryLang}
        onChange={(code) => {
          handleLangChange(code);
          close();
        }}
        onClose={close}
        autoSubtitle={autoSubtitle}
      />
    ),
  };

  const tabs: Array<[SideTab, string]> = [
    ['summary', t('tabSummary')],
    ['notes', t('tabNotes')],
    ['chat', t('tabChat')],
  ];

  return (
    <div className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden border-t border-[var(--af-border)] bg-[var(--af-bg)] md:border-t-0">
      <div role="tablist" className="flex shrink-0 items-end gap-5 border-b border-[var(--af-border)] px-5">
        {tabs.map(([value, label]) => (
          <button
            key={value}
            type="button"
            role="tab"
            aria-selected={tab === value}
            onClick={() => chooseTab(value)}
            className={`-mb-px border-b-2 py-2.5 text-sm transition-colors ${
              tab === value
                ? 'border-[var(--af-text)] font-medium text-[var(--af-text)]'
                : 'border-transparent text-[var(--af-text-3)] hover:text-[var(--af-text)]'
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Every tab stays mounted: the chat keeps its conversation and the
          summary its unsaved corrections while another tab is open. */}
      <div className={`min-h-0 flex-1 flex-col ${tab === 'summary' ? 'flex' : 'hidden'}`}>
      <div className="flex min-h-11 items-center gap-2 overflow-x-auto px-4 pb-1 pt-3">
        <div className="flex-shrink-0">
          <SummaryGeneratorButtonGroup
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            onSaveModelConfig={onSaveModelConfig}
            onGenerateSummary={onGenerateSummary}
            onRequestRegenerate={onRequestRegenerate}
            onStopGeneration={onStopGeneration}
            customPrompt={customPrompt}
            summaryStatus={summaryStatus}
            availableTemplates={availableTemplates}
            selectedTemplate={selectedTemplate}
            onTemplateSelect={onTemplateSelect}
            onManageTemplates={onManageTemplates}
            hasTranscripts={transcripts.length > 0}
            hasSummary={!!aiSummary}
            isModelConfigLoading={isModelConfigLoading}
            onOpenModelSettings={onOpenModelSettings}
            language={language}
          />
        </div>

        {aiSummary && (
          <div className="ml-auto flex-shrink-0">
            <SummaryUpdaterButtonGroup
              isSaving={isSaving}
              isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
              onSave={onSaveAll}
              onCopy={onCopySummary}
              onFind={() => {
                // TODO: Implement find in summary functionality
                console.log('Find in summary clicked');
              }}
              onOpenFolder={onOpenFolder}
              hasSummary={true}
            />
          </div>
        )}
      </div>

      <div className="flex-1 min-h-0">
        <InsightTabs
          aiSummary={aiSummary}
          transcripts={transcripts}
          generating={isSummaryLoading}
        />
      </div>
      </div>

      <div className={`min-h-0 flex-1 flex-col ${tab === 'notes' ? 'flex' : 'hidden'}`}>
        {/* A place kept for the owner's own notes, written during a recording. */}
        <div className="flex flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
          <NotebookPen size={20} className="text-[var(--af-text-3)]" />
          <p className="text-sm font-medium text-[var(--af-text-2)]">{t('notesSoonTitle')}</p>
          <p className="max-w-xs text-sm text-[var(--af-text-3)]">{t('notesSoonBody')}</p>
        </div>
      </div>

      <div className={`min-h-0 flex-1 flex-col ${tab === 'chat' ? 'flex' : 'hidden'}`}>
        <MeetingChat transcripts={transcripts} />
      </div>
    </div>
  );
}
