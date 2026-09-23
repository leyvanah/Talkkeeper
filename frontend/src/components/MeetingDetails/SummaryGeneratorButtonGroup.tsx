"use client";

/**
 * Making the summary: one button, and beside it what it is made with — the
 * template, the language and the model — in a small panel of their own, so
 * the toolbar is not four buttons in a row.
 */

import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import {
  ArrowLeft,
  Check,
  ChevronRight,
  Cpu,
  FileText,
  Languages,
  Loader2,
  SlidersHorizontal,
  Sparkles,
  Square,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useState, useEffect, useRef, ReactNode } from 'react';
import { useTranslations } from 'next-intl';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';

/** The summary's language, as the panel shows and changes it. */
export interface SummaryLanguageChoice {
  label: string;
  /** The picker, told how to close itself. */
  picker: (close: () => void) => ReactNode;
}

interface SummaryGeneratorButtonGroupProps {
  language?: SummaryLanguageChoice;
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  onGenerateSummary: (customPrompt: string) => Promise<boolean>;
  onRequestRegenerate?: () => void;
  onStopGeneration: () => void;
  customPrompt: string;
  summaryStatus: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
  availableTemplates: Array<{ id: string, name: string, description: string }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
  onManageTemplates?: () => void;
  hasTranscripts?: boolean;
  hasSummary?: boolean;
  isModelConfigLoading?: boolean;
  onOpenModelSettings?: (openFn: () => void) => void;
}

export function SummaryGeneratorButtonGroup({
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onGenerateSummary,
  onRequestRegenerate,
  onStopGeneration,
  customPrompt,
  summaryStatus,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
  onManageTemplates,
  hasTranscripts = true,
  hasSummary = false,
  isModelConfigLoading = false,
  onOpenModelSettings,
  language
}: SummaryGeneratorButtonGroupProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const [isCheckingModels, setIsCheckingModels] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  // The panel beside the button, and which of its pages is open.
  const [panelOpen, setPanelOpen] = useState(false);
  const [panelView, setPanelView] = useState<'main' | 'template' | 'language'>('main');
  // Regenerate-with-context popup: lets the user add one-off instructions
  // (e.g. "focus on action items", "keep it short") before regenerating.
  const [contextModalOpen, setContextModalOpen] = useState(false);
  const [contextInput, setContextInput] = useState('');

  // Expose the function to open the modal via callback registration
  useEffect(() => {
    if (onOpenModelSettings) {
      // Register our open dialog function with the parent by calling the callback
      // This allows the parent to store a reference to this function
      const openDialog = () => {
        console.log('📱 Opening model settings dialog via callback');
        setSettingsDialogOpen(true);
      };

      // Call the parent's callback with our open function
      // Note: This assumes onOpenModelSettings accepts a function parameter
      // We'll need to adjust the signature
      onOpenModelSettings(openDialog);
    }
  }, [onOpenModelSettings]);

  if (!hasTranscripts) {
    return null;
  }

  const checkBuiltInAIModelsAndGenerate = async (promptOverride?: string) => {
    const effectivePrompt = promptOverride !== undefined ? promptOverride : customPrompt;
    setIsCheckingModels(true);
    try {
      const selectedModel = modelConfig.model;

      // Check if specific model is configured
      if (!selectedModel) {
        toast.error(t('noBuiltinModelTitle'), {
          description: t('noBuiltinModelDescription'),
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Check model readiness (with filesystem refresh)
      const isReady = await invoke<boolean>('builtin_ai_is_model_ready', {
        modelName: selectedModel,
        refresh: true,
      });

      if (isReady) {
        // Model is available, proceed with generation
        onGenerateSummary(effectivePrompt);
        return;
      }

      // Model not ready - check detailed status
      const modelInfo = await invoke<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
        modelName: selectedModel,
      });

      if (!modelInfo) {
        toast.error(t('modelNotFoundTitle'), {
          description: t('modelNotFoundDescription', { model: selectedModel }),
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Handle different model states
      const status = modelInfo.status;

      if (status.type === 'downloading') {
        toast.info(t('modelDownloadingTitle'), {
          description: t('modelDownloadingDescription', { model: selectedModel, progress: status.progress }),
          duration: 5000,
        });
        return;
      }

      if (status.type === 'not_downloaded') {
        toast.error(t('modelNotDownloadedTitle'), {
          description: t('modelNotDownloadedDescription', { model: selectedModel }),
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      if (status.type === 'incomplete') {
        toast.info(t('modelIncompleteTitle'), {
          description: t('modelIncompleteDescription', { model: selectedModel }),
          duration: 7000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      if (status.type === 'corrupted') {
        toast.error(t('modelCorruptedTitle'), {
          description: t('modelCorruptedDescription', { model: selectedModel }),
          duration: 7000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      if (status.type === 'error') {
        toast.error(t('modelErrorTitle'), {
          description: status.Error || t('modelErrorDescription'),
          duration: 5000,
        });
        setSettingsDialogOpen(true);
        return;
      }

      // Fallback
      toast.error(t('modelNotAvailableTitle'), {
        description: t('modelNotAvailableDescription'),
        duration: 5000,
      });
      setSettingsDialogOpen(true);

    } catch (error) {
      console.error('Error checking built-in AI models:', error);
      toast.error(t('modelStatusCheckFailed'), {
        description: error instanceof Error ? error.message : String(error),
        duration: 5000,
      });
    } finally {
      setIsCheckingModels(false);
    }
  };

  const checkOllamaModelsAndGenerate = async (promptOverride?: string) => {
    const effectivePrompt = promptOverride !== undefined ? promptOverride : customPrompt;

    // Handle built-in AI provider
    if (modelConfig.provider === 'builtin-ai') {
      await checkBuiltInAIModelsAndGenerate(effectivePrompt);
      return;
    }

    // Only check for Ollama provider
    if (modelConfig.provider !== 'ollama') {
      onGenerateSummary(effectivePrompt);
      return;
    }

    setIsCheckingModels(true);
    try {
      const endpoint = modelConfig.ollamaEndpoint || null;
      const models = await invoke('get_ollama_models', { endpoint }) as any[];

      if (!models || models.length === 0) {
        // No models available, show message and open settings
        toast.error(t('ollamaNoModels'), { duration: 5000 });
        setSettingsDialogOpen(true);
        return;
      }

      // Models are available, proceed with generation
      onGenerateSummary(effectivePrompt);
    } catch (error) {
      console.error('Error checking Ollama models:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);

      if (isOllamaNotInstalledError(errorMessage)) {
        // Ollama is not installed - show specific message with download link
        toast.error(
          t('ollamaNotInstalledTitle'),
          {
            description: t('ollamaNotInstalledDescription'),
            duration: 7000,
            action: {
              label: t('ollamaDownloadAction'),
              onClick: () => invoke('open_external_url', { url: 'https://ollama.com/download' })
            }
          }
        );
      } else {
        // Other error - generic message
        toast.error(t('ollamaCheckFailed'), { duration: 5000 });
      }
      setSettingsDialogOpen(true);
    } finally {
      setIsCheckingModels(false);
    }
  };

  const isGenerating = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';

  // Called when the main button is clicked. For a first-time summary we generate
  // immediately; when regenerating an existing summary we first open a small
  // popup so the user can add one-off guidance for this run.
  const handlePrimaryClick = () => {
    if (hasSummary) {
      if (onRequestRegenerate) {
        onRequestRegenerate();
        return;
      }
      setContextInput('');
      setContextModalOpen(true);
    } else {
      checkOllamaModelsAndGenerate();
    }
  };

  // Regenerate using the base custom prompt plus the one-off context the user typed.
  const submitRegenerateWithContext = () => {
    const extra = contextInput.trim();
    const combined = [customPrompt.trim(), extra].filter(Boolean).join('\n\n');
    setContextModalOpen(false);
    checkOllamaModelsAndGenerate(combined);
  };

  const contextSuggestions = [
    t('suggestionActionItems'),
    t('suggestionShort'),
    t('suggestionDecisions'),
    t('suggestionFormalTone'),
  ];

  const templateName =
    availableTemplates.find((template) => template.id === selectedTemplate)?.name ?? selectedTemplate;
  const busy = isCheckingModels || isModelConfigLoading;

  const row =
    'flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm text-[var(--af-text)] transition-colors hover:bg-[var(--af-hover)]';
  const settingRow = (Icon: typeof FileText, label: string, value: string, onClick: () => void) => (
    <button type="button" className={row} onClick={onClick}>
      <Icon size={15} className="shrink-0 text-[var(--af-text-3)]" />
      <span className="shrink-0">{label}</span>
      <span className="ml-auto min-w-0 truncate text-[var(--af-text-3)]">{value}</span>
      <ChevronRight size={14} className="shrink-0 text-[var(--af-text-3)]" />
    </button>
  );

  return (
    <div className="flex items-center gap-1">
      {isGenerating ? (
        <button
          type="button"
          onClick={() => onStopGeneration()}
          title={t('stopSummaryGeneration')}
          aria-label={t('stopSummaryGeneration')}
          className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-red-500/40 px-3 text-sm font-medium text-red-500 transition-colors hover:bg-red-500/10"
        >
          <Square size={13} fill="currentColor" />
          {t('stop')}
        </button>
      ) : (
        <button
          type="button"
          onClick={handlePrimaryClick}
          disabled={busy}
          title={
            isModelConfigLoading
              ? t('loadingModelConfig')
              : isCheckingModels
                ? t('checkingModels')
                : hasSummary ? t('regenerateSummary') : t('generateSummary')
          }
          className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-[var(--af-accent)] px-3 text-sm font-medium text-[var(--af-accent-contrast)] transition-[filter] hover:brightness-110 disabled:opacity-60"
        >
          {busy ? <Loader2 className="animate-spin" size={15} /> : <Sparkles size={15} />}
          <span className="whitespace-nowrap">{hasSummary ? t('regenerateSummary') : t('generateSummary')}</span>
        </button>
      )}

      <Popover
        open={panelOpen}
        onOpenChange={(open) => {
          setPanelOpen(open);
          if (!open) setPanelView('main');
        }}
      >
        <PopoverTrigger asChild>
          <button
            type="button"
            title={t('summarySettings')}
            aria-label={t('summarySettings')}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]"
          >
            <SlidersHorizontal size={16} />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className={
            panelView === 'language'
              ? 'w-auto border-0 bg-transparent p-0 shadow-none'
              : 'w-72 border-[var(--af-border)] bg-[var(--af-panel)] p-1.5 text-[var(--af-text)]'
          }
        >
          {panelView === 'main' && (
            <div className="flex flex-col">
              <div className="px-2 pb-1 pt-1 text-xs text-[var(--af-text-3)]">{t('summarySettings')}</div>
              {(availableTemplates.length > 0 || onManageTemplates) &&
                settingRow(FileText, t('template'), templateName, () => setPanelView('template'))}
              {language &&
                settingRow(Languages, t('summaryLanguage'), language.label, () => setPanelView('language'))}
              {settingRow(Cpu, t('aiModel'), modelConfig.model || t('modelNotChosen'), () => {
                setPanelOpen(false);
                setPanelView('main');
                setSettingsDialogOpen(true);
              })}
            </div>
          )}

          {panelView === 'template' && (
            <div className="flex flex-col">
              <button
                type="button"
                onClick={() => setPanelView('main')}
                className="flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-[var(--af-text-3)] hover:text-[var(--af-text)]"
              >
                <ArrowLeft size={13} />
                {t('template')}
              </button>
              <div className="max-h-72 overflow-y-auto">
                {availableTemplates.map((template) => (
                  <button
                    key={template.id}
                    type="button"
                    title={template.description}
                    className={row}
                    onClick={() => {
                      onTemplateSelect(template.id, template.name);
                      setPanelView('main');
                    }}
                  >
                    <span className="min-w-0 flex-1 truncate">{template.name}</span>
                    {selectedTemplate === template.id && <Check size={14} className="shrink-0 text-[var(--af-accent)]" />}
                  </button>
                ))}
              </div>
              {onManageTemplates && (
                <button
                  type="button"
                  className={`${row} mt-1 border-t border-[var(--af-border)] text-[var(--af-text-2)]`}
                  onClick={() => {
                    setPanelOpen(false);
                    setPanelView('main');
                    onManageTemplates();
                  }}
                >
                  {t('manageTemplates')}
                </button>
              )}
            </div>
          )}

          {panelView === 'language' && language?.picker(() => setPanelView('main'))}
        </PopoverContent>
      </Popover>

      {/* The model's own settings are a dialog: they need the room. */}
      <Dialog open={settingsDialogOpen} onOpenChange={setSettingsDialogOpen}>
        <DialogContent aria-describedby={undefined}>
          <VisuallyHidden>
            <DialogTitle>{t('modelSettings')}</DialogTitle>
          </VisuallyHidden>
          <ModelSettingsModal
            onSave={async (config) => {
              await onSaveModelConfig(config);
              setSettingsDialogOpen(false);
            }}
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            skipInitialFetch={true}
            layout="dialog"
          />
        </DialogContent>
      </Dialog>

      {/* Regenerate-with-context popup */}
      <Dialog open={contextModalOpen} onOpenChange={setContextModalOpen}>
        <DialogContent aria-describedby={undefined} className="sm:max-w-lg">
          <DialogTitle className="flex items-center gap-2 text-base">
            <Sparkles size={18} className="text-blue-500" />
            {t('regenerateDialogTitle')}
          </DialogTitle>
          <div className="mt-2 space-y-3">
            <p className="text-sm text-gray-500">{t('regenerateDialogHint')}</p>
            <textarea
              autoFocus
              value={contextInput}
              onChange={(e) => setContextInput(e.target.value)}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                  e.preventDefault();
                  submitRegenerateWithContext();
                }
              }}
              placeholder={t('regenerateDialogPlaceholder')}
              rows={4}
              className="w-full resize-none rounded-md border border-[var(--af-border,#d1d5db)] bg-[var(--af-panel-2,#fff)] px-3 py-2 text-sm text-[var(--af-text,#111827)] outline-none focus:ring-2 focus:ring-blue-500"
            />
            <div className="flex flex-wrap gap-1.5">
              {contextSuggestions.map((s) => (
                <button
                  key={s}
                  type="button"
                  onClick={() =>
                    setContextInput((prev) => (prev.trim() ? `${prev.trim()}\n${s}` : s))
                  }
                  className="rounded-full border border-[var(--af-border,#e5e7eb)] px-2.5 py-1 text-xs text-gray-500 transition-colors hover:border-blue-400 hover:text-blue-500"
                >
                  + {s}
                </button>
              ))}
            </div>
          </div>
          <div className="mt-4 flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setContextModalOpen(false)}>
              {tc('cancel')}
            </Button>
            <Button
              size="sm"
              className="bg-gradient-to-r from-blue-600 to-purple-600 text-white hover:from-blue-700 hover:to-purple-700"
              onClick={submitRegenerateWithContext}
            >
              <Sparkles size={16} className="mr-1.5" />
              {t('regenerate')}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
