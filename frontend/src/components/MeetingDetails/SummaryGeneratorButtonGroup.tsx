"use client";

import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import {
  Dialog,
  DialogContent,
  DialogTrigger,
  DialogTitle,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Sparkles, Settings, Loader2, FileText, Check, Square } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useState, useEffect, useRef, ReactNode } from 'react';
import { useTranslations } from 'next-intl';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';

interface SummaryGeneratorButtonGroupProps {
  languageSlot?: ReactNode;
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
  languageSlot
}: SummaryGeneratorButtonGroupProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const [isCheckingModels, setIsCheckingModels] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
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

  return (
    <ButtonGroup>
      {/* Generate Summary or Stop button */}
      {isGenerating ? (
        <Button
          variant="outline"
          size="sm"
          className="bg-gradient-to-r from-red-50 to-orange-50 hover:from-red-100 hover:to-orange-100 border-red-200 xl:px-4"
          onClick={() => {
            onStopGeneration();
          }}
          title={t('stopSummaryGeneration')}
          aria-label={t('stopSummaryGeneration')}
        >
          <Square className="xl:mr-2" size={18} fill="currentColor" />
          <span className="hidden lg:inline xl:inline">{t('stop')}</span>
        </Button>
      ) : (
        <Button
          variant="outline"
          size="sm"
          className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 xl:px-4"
          onClick={handlePrimaryClick}
          disabled={isCheckingModels || isModelConfigLoading}
          title={
            isModelConfigLoading
              ? t('loadingModelConfig')
              : isCheckingModels
                ? t('checkingModels')
                : hasSummary ? t('regenerateSummary') : t('generateSummary')
          }
          aria-label={hasSummary ? t('regenerateSummary') : t('generateSummary')}
        >
          {isCheckingModels || isModelConfigLoading ? (
            <>
              <Loader2 className="animate-spin xl:mr-2" size={18} />
              <span className="hidden xl:inline">{t('processing')}</span>
            </>
          ) : (
            <>
              <Sparkles className="xl:mr-2" size={18} />
              <span className="hidden lg:inline xl:inline">{hasSummary ? t('regenerateSummary') : t('generateSummary')}</span>
            </>
          )}
        </Button>
      )}

      {languageSlot}

      {/* Settings button */}
      <Dialog open={settingsDialogOpen} onOpenChange={setSettingsDialogOpen}>
        <DialogTrigger asChild>
          <Button
            variant="outline"
            size="sm"
            title={t('summarySettings')}
            aria-label={t('summarySettings')}
          >
            <Settings />
            <span className="hidden lg:inline">{t('aiModel')}</span>
          </Button>
        </DialogTrigger>
        <DialogContent
          aria-describedby={undefined}
        >
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

      {/* Template selector dropdown */}
      {(availableTemplates.length > 0 || onManageTemplates) && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              title={t('selectTemplate')}
              aria-label={t('selectTemplate')}
            >
              <FileText />
              <span className="hidden lg:inline">{t('template')}</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {availableTemplates.map((template) => (
              <DropdownMenuItem
                key={template.id}
                onClick={() => onTemplateSelect(template.id, template.name)}
                title={template.description}
                className="flex items-center justify-between gap-2"
              >
                <span>{template.name}</span>
                {selectedTemplate === template.id && (
                  <Check className="h-4 w-4 text-green-600" />
                )}
              </DropdownMenuItem>
            ))}

            {onManageTemplates && (
              <DropdownMenuItem
                onClick={onManageTemplates}
                className="mt-1 border-t border-gray-100 font-medium text-blue-600"
              >
                {t('manageTemplates')}
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}

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
    </ButtonGroup>
  );
}
