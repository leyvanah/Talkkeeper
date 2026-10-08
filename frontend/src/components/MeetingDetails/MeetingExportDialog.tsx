"use client";

import { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { Clipboard, FileJson, FileText, FileType, Files, ScrollText } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import type { MeetingExportContent, MeetingExportFormat } from '@/hooks/meeting-details/useCopyOperations';

// Labels and descriptions are resolved per render so they follow the active locale;
// only the value/icon pairs are static.
const contentOptions: Array<{ value: MeetingExportContent; labelKey: string; descriptionKey: string }> = [
  { value: 'transcript', labelKey: 'exportContentTranscript', descriptionKey: 'exportContentTranscriptDescription' },
  { value: 'summary', labelKey: 'exportContentSummary', descriptionKey: 'exportContentSummaryDescription' },
  { value: 'both', labelKey: 'exportContentBoth', descriptionKey: 'exportContentBothDescription' },
];

const formatOptions: Array<{
  value: MeetingExportFormat;
  label: string;
  labelKey?: string;
  descriptionKey: string;
  icon: typeof FileText;
}> = [
  { value: 'pdf', label: 'PDF', descriptionKey: 'exportFormatPdfDescription', icon: FileText },
  { value: 'docx', label: 'Word', labelKey: 'exportFormatWord', descriptionKey: 'exportFormatWordDescription', icon: FileType },
  { value: 'txt', label: 'Text', labelKey: 'exportFormatText', descriptionKey: 'exportFormatTextDescription', icon: ScrollText },
  { value: 'markdown', label: 'Markdown', descriptionKey: 'exportFormatMarkdownDescription', icon: Files },
  { value: 'json', label: 'JSON', descriptionKey: 'exportFormatJsonDescription', icon: FileJson },
  { value: 'clipboard', label: 'Clipboard', labelKey: 'exportFormatClipboard', descriptionKey: 'exportFormatClipboardDescription', icon: Clipboard },
];

export function MeetingExportDialog({
  open,
  onOpenChange,
  hasTranscript,
  hasSummary,
  onExport,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  hasTranscript: boolean;
  hasSummary: boolean;
  onExport: (content: MeetingExportContent, format: MeetingExportFormat) => Promise<boolean>;
}) {
  const t = useTranslations('meetingDetails');
  const [step, setStep] = useState<'content' | 'format'>('content');
  const [content, setContent] = useState<MeetingExportContent>('both');
  const [exporting, setExporting] = useState(false);

  useEffect(() => {
    if (!open) return;
    setStep('content');
    setContent(hasTranscript && hasSummary ? 'both' : hasTranscript ? 'transcript' : 'summary');
    setExporting(false);
  }, [open, hasTranscript, hasSummary]);

  const isAvailable = (value: MeetingExportContent) => {
    if (value === 'transcript') return hasTranscript;
    if (value === 'summary') return hasSummary;
    return hasTranscript && hasSummary;
  };

  const exportAs = async (format: MeetingExportFormat) => {
    setExporting(true);
    const succeeded = await onExport(content, format);
    setExporting(false);
    if (succeeded) onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => !exporting && onOpenChange(nextOpen)}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{step === 'content' ? t('exportStepContentTitle') : t('exportStepFormatTitle')}</DialogTitle>
          <DialogDescription>
            {step === 'content'
              ? t('exportStepContentDescription')
              : t('exportStepFormatDescription', {
                  what:
                    content === 'both'
                      ? t('exportWhatBoth')
                      : content === 'transcript'
                        ? t('exportWhatTranscript')
                        : t('exportWhatSummary'),
                })}
          </DialogDescription>
        </DialogHeader>

        {step === 'content' ? (
          <div className="grid gap-2">
            {contentOptions.map((option) => (
              <Button
                key={option.value}
                type="button"
                variant={content === option.value ? 'default' : 'outline'}
                className="h-auto justify-start px-4 py-3 text-left"
                disabled={!isAvailable(option.value)}
                onClick={() => setContent(option.value)}
              >
                <span>
                  <span className="block text-sm font-semibold">{t(option.labelKey)}</span>
                  <span className="block text-xs font-normal opacity-70">
                    {isAvailable(option.value) ? t(option.descriptionKey) : t('exportNotAvailable')}
                  </span>
                </span>
              </Button>
            ))}
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-2">
            {formatOptions.map((option) => {
              const Icon = option.icon;
              return (
                <Button
                  key={option.value}
                  type="button"
                  variant="outline"
                  className="h-auto justify-start px-3 py-3 text-left"
                  disabled={exporting}
                  onClick={() => void exportAs(option.value)}
                >
                  <Icon size={17} />
                  <span>
                    <span className="block text-sm font-semibold">{option.labelKey ? t(option.labelKey) : option.label}</span>
                    <span className="block text-[11px] font-normal opacity-70">{t(option.descriptionKey)}</span>
                  </span>
                </Button>
              );
            })}
          </div>
        )}

        {step === 'format' && (
          <p className="text-xs text-amber-600">{t('exportUnencryptedNote')}</p>
        )}

        <DialogFooter>
          {step === 'format' && (
            <Button type="button" variant="outline" disabled={exporting} onClick={() => setStep('content')}>
              {t('exportBack')}
            </Button>
          )}
          {step === 'content' && (
            <Button type="button" disabled={!isAvailable(content)} onClick={() => setStep('format')}>
              {t('exportChooseFormat')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
