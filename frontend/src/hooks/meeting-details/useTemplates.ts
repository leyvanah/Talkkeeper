import { useState, useEffect, useCallback } from 'react';
import { useTranslations } from 'next-intl';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';

export function useTemplates() {
  const t = useTranslations('meetingDetails');
  const [availableTemplates, setAvailableTemplates] = useState<Array<{
    id: string;
    name: string;
    description: string;
  }>>([]);
  const [selectedTemplate, setSelectedTemplate] = useState<string>('standard_meeting');

  const refreshTemplates = useCallback(async () => {
    try {
      const templates = await invokeTauri('api_list_templates') as Array<{
        id: string;
        name: string;
        description: string;
      }>;
      setAvailableTemplates(templates);
      return templates;
    } catch (error) {
      console.error('Failed to fetch templates:', error);
      return [];
    }
  }, []);

  // Fetch available templates on mount
  useEffect(() => {
    void refreshTemplates();
  }, [refreshTemplates]);

  // Create or overwrite a custom template. `templateJson` must match the template schema.
  const saveCustomTemplate = useCallback(async (templateId: string, templateJson: string) => {
    const savedId = await invokeTauri('api_save_custom_template', {
      templateId,
      templateJson,
    }) as string;
    await refreshTemplates();
    return savedId;
  }, [refreshTemplates]);

  // Delete a custom template (built-ins are not deletable).
  const deleteCustomTemplate = useCallback(async (templateId: string) => {
    await invokeTauri('api_delete_custom_template', { templateId });
    await refreshTemplates();
    if (selectedTemplate === templateId) setSelectedTemplate('standard_meeting');
  }, [refreshTemplates, selectedTemplate]);

  const isCustomTemplate = useCallback(async (templateId: string): Promise<boolean> => {
    try {
      return await invokeTauri('api_is_custom_template', { templateId }) as boolean;
    } catch {
      return false;
    }
  }, []);

  // Handle template selection
  const handleTemplateSelection = useCallback((templateId: string, templateName: string) => {
    setSelectedTemplate(templateId);
    toast.success(t('templateSelected'), {
      description: t('templateSelectedDescription', { name: templateName }),
    });
  }, [t]);

  return {
    availableTemplates,
    selectedTemplate,
    handleTemplateSelection,
    refreshTemplates,
    saveCustomTemplate,
    deleteCustomTemplate,
    isCustomTemplate,
  };
}
