import { useState, useCallback, useRef, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { Transcript, Summary } from '@/types';
import { BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { CurrentMeeting, useSidebar } from '@/components/Sidebar/SidebarProvider';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { runSaveSteps, SaveStep } from '@/lib/save-meeting-changes';

interface UseMeetingDataProps {
  meeting: any;
  summaryData: Summary | null;
  onMeetingUpdated?: () => Promise<void>;
}

export function useMeetingData({ meeting, summaryData, onMeetingUpdated }: UseMeetingDataProps) {
  const t = useTranslations('meetingDetails');
  const ts = useTranslations('sidebar');

  // State
  // Use prop directly since summary generation fetches transcripts independently
  const transcripts = meeting.transcripts;
  const [meetingTitle, setMeetingTitle] = useState(meeting.title || `+ ${ts('newCall')}`);
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [isTitleDirty, setIsTitleDirty] = useState(false);
  const [aiSummary, setAiSummary] = useState<Summary | null>(summaryData);
  const [isSaving, setIsSaving] = useState(false);
  const [, setIsSummaryDirty] = useState(false);

  // Ref for BlockNoteSummaryView
  const blockNoteSummaryRef = useRef<BlockNoteSummaryViewRef>(null);

  // Sidebar context
  const { currentMeeting, setCurrentMeeting, setMeetings, meetings: sidebarMeetings } = useSidebar();

  // Sync from the parent prop when it actually has a summary. Never clobber a
  // locally-generated summary with `null` — that was wiping the just-finished
  // auto-summary whenever the parent re-rendered with meetingSummary still null
  // (the parent only loads summary once on meetingId change, not after generate).
  useEffect(() => {
    if (summaryData) {
      console.log('[useMeetingData] Syncing summary data from prop');
      setAiSummary(summaryData);
    }
  }, [summaryData]);

  // When the meeting changes, reset to whatever the parent loaded for it.
  useEffect(() => {
    setAiSummary(summaryData);
    setMeetingTitle(meeting.title || '');
    setIsTitleDirty(false);
  }, [meeting.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Sidebar renames are persisted outside this hook. Mirror them without
  // resetting summary state or destroying an unsaved local title edit.
  useEffect(() => {
    if (currentMeeting && currentMeeting.id === meeting.id && !isTitleDirty) {
      setMeetingTitle(currentMeeting.title || '');
    }
  }, [currentMeeting?.id, currentMeeting?.title, meeting.id, isTitleDirty]);

  // Handlers
  const handleTitleChange = useCallback((newTitle: string) => {
    setMeetingTitle(newTitle);
    setIsTitleDirty(true);
  }, []);

  const handleSummaryChange = useCallback((newSummary: Summary) => {
    setAiSummary(newSummary);
  }, []);

  // Throws when the title is not saved, so the caller cannot report success.
  const handleSaveMeetingTitle = useCallback(async () => {
    await invokeTauri('api_save_meeting_title', {
      meetingId: meeting.id,
      title: meetingTitle,
    });

    console.log('Save meeting title success');
    setIsTitleDirty(false);

    // Update meetings with new title
    const updatedMeetings = sidebarMeetings.map((m: CurrentMeeting) =>
      m.id === meeting.id ? { id: m.id, title: meetingTitle } : m
    );
    setMeetings(updatedMeetings);
    setCurrentMeeting({ id: meeting.id, title: meetingTitle });
  }, [meeting.id, meetingTitle, sidebarMeetings, setMeetings, setCurrentMeeting]);

  // Throws when the summary is not saved, so the caller cannot report success.
  const handleSaveSummary = useCallback(async (summary: Summary | { markdown?: string; summary_json?: any[] }) => {
    console.log('📄 handleSaveSummary called with:', {
      hasMarkdown: 'markdown' in summary,
      hasSummaryJson: 'summary_json' in summary,
      summaryKeys: Object.keys(summary)
    });

    let formattedSummary: any;

    // Check if it's the new BlockNote format
    if ('markdown' in summary || 'summary_json' in summary) {
      console.log('📄 Saving new format (markdown/blocknote)');
      formattedSummary = summary;
    } else {
      console.log('📄 Saving legacy format');
      formattedSummary = {
        MeetingName: meetingTitle,
        MeetingNotes: {
          sections: Object.entries(summary).map(([, section]) => ({
            title: section.title,
            blocks: section.blocks
          }))
        }
      };
    }

    await invokeTauri('api_save_meeting_summary', {
      meetingId: meeting.id,
      summary: formattedSummary,
    });

    console.log('✅ Save meeting summary success');
  }, [meeting.id, meetingTitle]);

  const saveAllChanges = useCallback(async () => {
    setIsSaving(true);
    try {
      const steps: SaveStep[] = [];

      // Save meeting title only if changed
      if (isTitleDirty) {
        steps.push({ part: 'title', run: handleSaveMeetingTitle });
      }

      // Save BlockNote editor changes if dirty
      const editor = blockNoteSummaryRef.current;
      if (editor?.isDirty) {
        console.log('💾 Saving BlockNote editor changes...');
        steps.push({ part: 'summary', run: () => editor.saveSummary() });
      } else if (aiSummary) {
        steps.push({ part: 'summary', run: () => handleSaveSummary(aiSummary) });
      }

      const failed = await runSaveSteps(steps);
      if (failed.length === 0) {
        toast.success(t('changesSaved'));
      } else {
        // Name what was not saved rather than echo the backend's (English) error;
        // the error itself is in the console.
        toast.error(t('changesSaveFailed'), {
          description: failed
            .map((part) => t(part === 'title' ? 'titleNotSaved' : 'summaryNotSaved'))
            .join(' '),
        });
      }
    } finally {
      setIsSaving(false);
    }
  }, [isTitleDirty, handleSaveMeetingTitle, aiSummary, handleSaveSummary, t]);

  return {
    // State
    transcripts,
    meetingTitle,
    isEditingTitle,
    isTitleDirty,
    aiSummary,
    isSaving,
    blockNoteSummaryRef,

    // Setters
    setMeetingTitle,
    setIsEditingTitle,
    setAiSummary,
    setIsSummaryDirty,

    // Handlers
    handleTitleChange,
    handleSummaryChange,
    handleSaveSummary,
    handleSaveMeetingTitle,
    saveAllChanges,
  };
}
