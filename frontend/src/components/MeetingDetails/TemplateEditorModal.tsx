'use client';

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { toast } from 'sonner';
import { toastFailure } from '@/lib/failure';

type Format = 'paragraph' | 'list' | 'table' | 'string';

interface SectionDraft {
  title: string;
  instruction: string;
  format: Format;
  item_format?: string;
}

interface TemplateEditorModalProps {
  open: boolean;
  onClose: () => void;
  availableTemplates: Array<{ id: string; name: string; description: string }>;
  onSave: (templateId: string, templateJson: string) => Promise<string>;
  onDelete: (templateId: string) => Promise<void>;
}

function slugify(name: string): string {
  const ascii = name.toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 60);
  if (ascii) return ascii;
  // Non-Latin names (e.g. Russian) slugify to nothing, so derive a stable ASCII
  // id from the name instead of rejecting it.
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return 'template_' + h.toString(36);
}

const emptySection = (): SectionDraft => ({ title: '', instruction: '', format: 'list' });

export function TemplateEditorModal({
  open,
  onClose,
  availableTemplates,
  onSave,
  onDelete,
}: TemplateEditorModalProps) {
  const t = useTranslations('meetingDetails');
  const tc = useTranslations('common');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [sections, setSections] = useState<SectionDraft[]>([emptySection()]);
  const [saving, setSaving] = useState(false);

  if (!open) return null;

  const reset = () => {
    setName('');
    setDescription('');
    setSections([emptySection()]);
  };

  const updateSection = (i: number, patch: Partial<SectionDraft>) => {
    setSections((prev) => prev.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  };

  const save = async () => {
    if (!name.trim()) return toast.error(t('templateNameRequired'));
    if (!description.trim()) return toast.error(t('templateDescriptionRequired'));
    const cleaned = sections
      .map((s) => ({ ...s, title: s.title.trim(), instruction: s.instruction.trim() }))
      .filter((s) => s.title && s.instruction);
    if (cleaned.length === 0) return toast.error(t('templateSectionRequired'));

    const template = {
      name: name.trim(),
      description: description.trim(),
      sections: cleaned.map((s) => {
        const out: any = { title: s.title, instruction: s.instruction, format: s.format };
        if (s.item_format && s.item_format.trim()) out.item_format = s.item_format.trim();
        return out;
      }),
    };

    const id = slugify(name);

    setSaving(true);
    try {
      await onSave(id, JSON.stringify(template, null, 2));
      toast.success(t('templateSaved', { name }));
      reset();
      onClose();
    } catch (e) {
      toastFailure(t('templateSaveFailed'), 'template-save', e);
    } finally {
      setSaving(false);
    }
  };

  const del = async (id: string, tname: string) => {
    try {
      await onDelete(id);
      toast.success(t('templateDeleted', { name: tname }));
    } catch (e) {
      // Built-in templates aren't deletable and the backend returns "not found"
      if (typeof e === 'string' && e.includes('not found')) {
        toast.error(t('templateBuiltInNotDeletable'));
      } else {
        toastFailure(t('templateDeleteFailed'), 'template-delete', e);
      }
    }
  };

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div
        className="flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl bg-white shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-gray-100 px-5 py-3">
          <h2 className="text-lg font-semibold text-gray-800">{t('templatesTitle')}</h2>
          <button onClick={onClose} className="rounded px-2 py-1 text-gray-500 hover:bg-gray-100">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          {/* Create form */}
          <div className="space-y-3">
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div>
                <label className="mb-1 block text-xs font-medium text-gray-600">{t('templateNameLabel')}</label>
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t('templateNamePlaceholder')}
                  className="w-full rounded-lg border border-gray-200 px-2 py-1.5 text-sm focus:border-blue-400 focus:outline-none"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-gray-600">{t('templateDescriptionLabel')}</label>
                <input
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={t('templateDescriptionPlaceholder')}
                  className="w-full rounded-lg border border-gray-200 px-2 py-1.5 text-sm focus:border-blue-400 focus:outline-none"
                />
              </div>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-gray-600">{t('templateSections')}</label>
                <button
                  onClick={() => setSections((p) => [...p, emptySection()])}
                  className="rounded px-2 py-1 text-xs font-medium text-blue-600 hover:bg-blue-50"
                >
                  {t('templateAddSection')}
                </button>
              </div>

              {sections.map((s, i) => (
                <div key={i} className="rounded-lg border border-gray-200 p-3">
                  <div className="mb-2 flex items-center gap-2">
                    <input
                      value={s.title}
                      onChange={(e) => updateSection(i, { title: e.target.value })}
                      placeholder={t('templateSectionTitlePlaceholder')}
                      className="flex-1 rounded border border-gray-200 px-2 py-1 text-sm focus:border-blue-400 focus:outline-none"
                    />
                    <select
                      value={s.format}
                      onChange={(e) => updateSection(i, { format: e.target.value as Format })}
                      className="rounded border border-gray-200 px-2 py-1 text-sm"
                      title={t('templateFormatTitle')}
                    >
                      <option value="list">{t('templateFormatList')}</option>
                      <option value="paragraph">{t('templateFormatParagraph')}</option>
                      <option value="table">{t('templateFormatTable')}</option>
                      <option value="string">{t('templateFormatString')}</option>
                    </select>
                    {sections.length > 1 && (
                      <button
                        onClick={() => setSections((p) => p.filter((_, idx) => idx !== i))}
                        className="rounded px-2 py-1 text-xs text-red-500 hover:bg-red-50"
                        title={t('templateRemoveSection')}
                      >
                        ✕
                      </button>
                    )}
                  </div>
                  <textarea
                    value={s.instruction}
                    onChange={(e) => updateSection(i, { instruction: e.target.value })}
                    rows={2}
                    placeholder={t('templateInstructionPlaceholder')}
                    className="w-full resize-none rounded border border-gray-200 px-2 py-1 text-sm focus:border-blue-400 focus:outline-none"
                  />
                </div>
              ))}
            </div>

            <div className="flex justify-end gap-2 pt-1">
              <button onClick={reset} className="rounded-lg px-3 py-2 text-sm text-gray-600 hover:bg-gray-100">
                {tc('clear')}
              </button>
              <button
                onClick={save}
                disabled={saving}
                className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white disabled:bg-gray-300"
              >
                {saving ? t('templateSaving') : t('templateSave')}
              </button>
            </div>
          </div>

          {/* Existing templates */}
          <div className="mt-6 border-t border-gray-100 pt-4">
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-500">
              {t('templateYourTemplates')}
            </h3>
            <div className="space-y-1">
              {availableTemplates.map((tpl) => (
                <div key={tpl.id} className="flex items-center justify-between rounded-lg px-2 py-1.5 hover:bg-gray-50">
                  <div className="min-w-0">
                    <div className="truncate text-sm text-gray-800">{tpl.name}</div>
                    <div className="truncate text-xs text-gray-400">{tpl.description}</div>
                  </div>
                  <button
                    onClick={() => del(tpl.id, tpl.name)}
                    className="ml-3 shrink-0 rounded px-2 py-1 text-xs text-red-500 hover:bg-red-50"
                    title={t('templateDeleteTitle')}
                  >
                    {tc('delete')}
                  </button>
                </div>
              ))}
            </div>
            <p className="mt-2 text-xs text-gray-400">{t('templateBuiltInNote')}</p>
          </div>
        </div>
      </div>
    </div>
  );
}

export default TemplateEditorModal;
