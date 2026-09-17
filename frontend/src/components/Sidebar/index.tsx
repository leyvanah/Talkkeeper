'use client';

/**
 * Primary left navigation sidebar.
 *
 * Layout (top → bottom): brand ("Talkkeeper", see Logo.tsx),
 * a global-search trigger (Ctrl/Cmd+K), a teal "New Recording" action, the
 * library tree, and a Settings-only footer.
 *
 * The tree has one collapsible folder per client, newest-seen first, followed
 * by a folder holding everything not filed under anyone — which is where fresh
 * recordings land. Only the unassigned folder is capped by RECENT_LIMIT: it is
 * the one that grows without bound, while a client folder is a whole history
 * worth seeing at once. Expanded folders are remembered across restarts.
 *
 * Supports shift/ctrl multi-select + bulk delete of meetings.
 *
 * State/wiring:
 *  - Reads the meetings list + current meeting + recording status from
 *    SidebarProvider (useSidebar) — the single source of truth kept in sync
 *    with the Rust core via Tauri commands/events.
 *  - Navigation uses next/navigation; selecting a meeting routes to
 *    /meeting-details?id=...  (see app/meeting-details/page-content.tsx).
 *  - Default state is expanded (isCollapsed=false in SidebarProvider).
 */

import React, { useState, useMemo, useEffect } from 'react';
import { ChevronDown, ChevronRight, FileText, AudioLines, ArrowRight, Settings, Trash2, Mic, Square, Plus, Search, Pencil, NotebookPen, Upload, User, Users, FolderInput, Inbox } from 'lucide-react';
import { useTranslations } from 'next-intl';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar, UNASSIGNED_FOLDER_ID, clientFolderId } from './SidebarProvider';
import { ClientPickerDialog } from '@/components/ClientPickerDialog';
import type { CurrentMeeting, SidebarItem } from '@/components/Sidebar/SidebarProvider';
import { ConfirmationModal } from '../ConfirmationModel/confirmation-modal';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { TranscriptModelProps } from '@/components/TranscriptSettings';
import Analytics from '@/lib/analytics';
import { invoke } from '@tauri-apps/api/core';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';

import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog"
import { VisuallyHidden } from "@/components/ui/visually-hidden"

import { MessageToast } from '../MessageToast';
import Logo from '../Logo';
import { ResizeHandle } from '../ResizeHandle';
import { setSidebarOffsetVariable, usePanelLayout } from '../PanelLayoutProvider';
import {
  SIDEBAR_COLLAPSED_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  sidebarFromDrag,
} from '@/lib/panel-layout';
import { ComplianceNotification } from '../ComplianceNotification';

/** Which folders were left open, so the tree looks the same after a restart. */
const EXPANDED_FOLDERS_KEY = 'meetily_expanded_folders';

function readExpandedFolders(): Set<string> | null {
  try {
    const stored = localStorage.getItem(EXPANDED_FOLDERS_KEY);
    if (!stored) return null;
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed) ? new Set(parsed.filter((id: unknown) => typeof id === 'string')) : null;
  } catch {
    return null;
  }
}

function formatDurationShort(secs?: number): string {
  if (secs == null || !Number.isFinite(secs) || secs <= 0) return '';
  const total = Math.round(secs);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s.toString().padStart(2, '0')}s`;
  return `${s}s`;
}

// "RECENT MEETINGS" rows show a date/time subtitle. Meetings created before the
// timestamp was tracked fall back to parsing it out of the auto-generated title
// (e.g. "Meeting 2026-08-05_21-59-55").
function parseMeetingDate(item: { createdAt?: string; title?: string }): Date | null {
  if (item.createdAt) {
    const d = new Date(item.createdAt);
    if (!isNaN(d.getTime())) return d;
  }
  const m = item.title?.match(/(\d{4})-(\d{2})-(\d{2})[_ T](\d{2})[-:](\d{2})(?:[-:](\d{2}))?/);
  if (m) {
    const [, y, mo, da, h, mi, s] = m;
    const d = new Date(Number(y), Number(mo) - 1, Number(da), Number(h), Number(mi), Number(s || '0'));
    if (!isNaN(d.getTime())) return d;
  }
  return null;
}

function formatMeetingDate(d: Date): string {
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

function formatMeetingTime(d: Date): string {
  return d.toLocaleString(undefined, { hour: 'numeric', minute: '2-digit' });
}

const Sidebar: React.FC = () => {
  const t = useTranslations('sidebar');
  const tc = useTranslations('common');
  const router = useRouter();
  const pathname = usePathname();
  const {
    currentMeeting,
    setCurrentMeeting,
    sidebarItems,
    isCollapsed,
    toggleCollapse,
    handleRecordingToggle,
    meetings,
    setMeetings,
    serverAddress,
    clients,
    refetchClients,
    refetchMeetings,
  } = useSidebar();
  const { layout: panelLayout, setSidebarWidth, collapseSidebar } = usePanelLayout();

  // Dragging the sidebar's edge: the width is previewed on the page itself,
  // and only collapsing (which changes what the sidebar shows) goes through
  // React while the pointer is still down.
  const dragSidebarTo = (x: number, commit: boolean) => {
    const next = sidebarFromDrag(x);
    if (next.collapsed !== panelLayout.sidebarCollapsed) collapseSidebar(next.collapsed);
    if (next.collapsed) {
      setSidebarOffsetVariable(SIDEBAR_COLLAPSED_WIDTH);
      return;
    }
    setSidebarOffsetVariable(next.width);
    if (commit) setSidebarWidth(next.width);
  };

  // Get recording state from RecordingStateContext (single source of truth)
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();
  // Unassigned is open on a fresh profile: on an empty archive it is the only
  // folder with anything in it, and an all-collapsed tree reads as "no data".
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set([UNASSIGNED_FOLDER_ID]));
  const [showModelSettings, setShowModelSettings] = useState(false);
  const [modelConfig, setModelConfig] = useState<ModelConfig>({
    provider: 'ollama',
    model: '',
    whisperModel: '',
    apiKey: null,
    ollamaEndpoint: null
  });
  const [transcriptModelConfig, setTranscriptModelConfig] = useState<TranscriptModelProps>({
    provider: 'parakeet',
    model: 'parakeet-tdt-0.6b-v3-int8',
  });
  const [settingsSaveSuccess, setSettingsSaveSuccess] = useState<boolean | null>(null);

  // State for edit modal
  const [editModalState, setEditModalState] = useState<{ isOpen: boolean; meetingId: string | null; currentTitle: string }>({
    isOpen: false,
    meetingId: null,
    currentTitle: ''
  });
  const [editingTitle, setEditingTitle] = useState<string>('');

  // Restore the open folders once, after mount: localStorage is not there
  // during the static export's prerender.
  useEffect(() => {
    const stored = readExpandedFolders();
    if (stored) setExpandedFolders(stored);
  }, []);

  // useEffect(() => {
  //   if (settingsSaveSuccess !== null) {
  //     const timer = setTimeout(() => {
  //       setSettingsSaveSuccess(null);
  //     }, 3000);
  //   }
  // }, [settingsSaveSuccess]);


  const [deleteModalState, setDeleteModalState] = useState<{ isOpen: boolean; itemId: string | null }>({ isOpen: false, itemId: null });

  // Multi-select for the meeting list: shift-click selects a range, ctrl/cmd
  // click toggles one, and the selection can be deleted in bulk.
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [lastSelectedId, setLastSelectedId] = useState<string | null>(null);
  const [bulkDeleteOpen, setBulkDeleteOpen] = useState(false);

  // The unassigned folder shows the newest few with a "View all" toggle.
  const RECENT_LIMIT = 8;
  const [showAllMeetings, setShowAllMeetings] = useState(false);

  // Client folders: create, rename, delete, and filing a recording under one.
  const [clientDialog, setClientDialog] = useState<{ mode: 'create' | 'rename'; clientId: string | null; name: string } | null>(null);
  const [clientDeleteState, setClientDeleteState] = useState<{ id: string; name: string; meetingCount: number } | null>(null);
  const [assignDialogMeetingId, setAssignDialogMeetingId] = useState<string | null>(null);

  const normalizedNewName = clientDialog?.name.trim().replace(/\s+/g, ' ').toLowerCase() ?? '';
  const duplicateClientName = normalizedNewName.length > 0 && clients.some(client =>
    client.id !== clientDialog?.clientId &&
    client.displayName.trim().replace(/\s+/g, ' ').toLowerCase() === normalizedNewName
  );

  useEffect(() => {
    // Note: Don't set hardcoded defaults - let DB be the source of truth
    const fetchModelConfig = async () => {
      // Only make API call if serverAddress is loaded
      if (!serverAddress) {
        console.log('Waiting for server address to load before fetching model config');
        return;
      }

      try {
        const data = await invoke('api_get_model_config') as any;
        if (data && data.provider !== null) {
          // Fetch API key if not included and provider requires it
          if (data.provider !== 'ollama' && !data.apiKey) {
            try {
              const apiKeyData = await invoke('api_get_api_key', {
                provider: data.provider
              }) as string;
              data.apiKey = apiKeyData;
            } catch (err) {
              console.error('Failed to fetch API key:', err);
            }
          }
          setModelConfig(data);
        }
      } catch (error) {
        console.error('Failed to fetch model config:', error);
      }
    };

    fetchModelConfig();
  }, [serverAddress]);


  useEffect(() => {
    // Note: Don't set hardcoded defaults - let DB be the source of truth
    const fetchTranscriptSettings = async () => {
      // Only make API call if serverAddress is loaded
      if (!serverAddress) {
        console.log('Waiting for server address to load before fetching transcript settings');
        return;
      }

      try {
        const data = await invoke('api_get_transcript_config') as any;
        if (data && data.provider !== null) {
          setTranscriptModelConfig(data);
        }
      } catch (error) {
        console.error('Failed to fetch transcript settings:', error);
      }
    };
    fetchTranscriptSettings();
  }, [serverAddress]);

  // Listen for model config updates from other components
  useEffect(() => {
    const setupListener = async () => {
      const { listen } = await import('@tauri-apps/api/event');
      const unlisten = await listen<ModelConfig>('model-config-updated', (event) => {
        console.log('Sidebar received model-config-updated event:', event.payload);
        setModelConfig(event.payload);
      });

      return unlisten;
    };

    let cleanup: (() => void) | undefined;
    setupListener().then(fn => cleanup = fn);

    return () => {
      cleanup?.();
    };
  }, []);



  // Handle model config save
  const handleSaveModelConfig = async (config: ModelConfig) => {
    try {
      await invoke('api_save_model_config', {
        provider: config.provider,
        model: config.model,
        whisperModel: config.whisperModel,
        apiKey: config.apiKey,
        ollamaEndpoint: config.ollamaEndpoint,
      });

      setModelConfig(config);
      console.log('Model config saved successfully');
      setSettingsSaveSuccess(true);

      // Emit event to sync other components
      const { emit } = await import('@tauri-apps/api/event');
      await emit('model-config-updated', config);

      // Track settings change
      await Analytics.trackSettingsChanged('model_config', `${config.provider}_${config.model}`);
    } catch (error) {
      console.error('Error saving model config:', error);
      setSettingsSaveSuccess(false);
    }
  };

  const handleSaveTranscriptConfig = async (updatedConfig?: TranscriptModelProps) => {
    try {
      const configToSave = updatedConfig || transcriptModelConfig;
      const payload = {
        provider: configToSave.provider,
        model: configToSave.model,
        apiKey: configToSave.apiKey ?? null
      };
      console.log('Saving transcript config with payload:', payload);

      await invoke('api_save_transcript_config', {
        provider: payload.provider,
        model: payload.model,
        apiKey: payload.apiKey,
      });


      setSettingsSaveSuccess(true);

      // Track settings change
      const transcriptConfigToSave = updatedConfig || transcriptModelConfig;
      await Analytics.trackSettingsChanged('transcript_config', `${transcriptConfigToSave.provider}_${transcriptConfigToSave.model}`);
    } catch (error) {
      console.error('Failed to save transcript config:', error);
      setSettingsSaveSuccess(false);
    }
  };

  const openGlobalSearch = () => window.dispatchEvent(new CustomEvent('open-global-search'));


  const handleDelete = async (itemId: string) => {
    console.log('Deleting item:', itemId);
    const payload = {
      meetingId: itemId
    };

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('api_delete_meeting', {
        meetingId: itemId,
      });
      console.log('Meeting deleted successfully');
      const updatedMeetings = meetings.filter((m: CurrentMeeting) => m.id !== itemId);
      setMeetings(updatedMeetings);
      // The client folder's count and "last seen" moved with it.
      await refetchClients();

      // Track meeting deletion
      Analytics.trackMeetingDeleted(itemId);

      // Show success toast
      toast.success(t('meetingDeletedSuccess'), {
        description: t('dataRemoved')
      });

      // If deleting the active meeting, navigate to home
      if (currentMeeting?.id === itemId) {
        setCurrentMeeting({ id: 'intro-call', title: '+ ' + t('newCall') });
        router.push('/');
      }
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      toast.error(t('deleteMeetingFailed'), {
        description: error instanceof Error ? error.message : String(error)
      });
    }
  };

  const handleDeleteConfirm = () => {
    if (deleteModalState.itemId) {
      handleDelete(deleteModalState.itemId);
    }
    setDeleteModalState({ isOpen: false, itemId: null });
  };

  // Flat, ordered list of meeting ids as currently displayed — needed so a
  // shift-click can select the contiguous range between two clicks.
  const orderedMeetingIds = useMemo(() => {
    const ids: string[] = [];
    const walk = (items: SidebarItem[]) => {
      for (const item of items) {
        if (item.type === 'folder') {
          // A collapsed folder is not on screen, so a shift-click range must
          // not silently reach through it.
          if (item.children && expandedFolders.has(item.id)) walk(item.children);
        } else if (item.id.includes('-') && !item.id.startsWith('intro-call')) {
          ids.push(item.id);
        }
      }
    };
    walk(sidebarItems);
    return ids;
  }, [sidebarItems, expandedFolders]);

  const clearSelection = () => {
    setSelectedIds(new Set());
    setLastSelectedId(null);
  };

  const handleMeetingSelect = (id: string, e: React.MouseEvent) => {
    setSelectedIds(prev => {
      const next = new Set(prev);
      if (e.shiftKey && lastSelectedId) {
        const a = orderedMeetingIds.indexOf(lastSelectedId);
        const b = orderedMeetingIds.indexOf(id);
        if (a !== -1 && b !== -1) {
          const [lo, hi] = a < b ? [a, b] : [b, a];
          for (let i = lo; i <= hi; i++) next.add(orderedMeetingIds[i]);
        } else {
          next.has(id) ? next.delete(id) : next.add(id);
        }
      } else {
        next.has(id) ? next.delete(id) : next.add(id);
      }
      return next;
    });
    setLastSelectedId(id);
  };

  const handleBulkDelete = async () => {
    const ids = Array.from(selectedIds);
    let ok = 0;
    for (const id of ids) {
      try {
        await invoke('api_delete_meeting', { meetingId: id });
        Analytics.trackMeetingDeleted(id);
        ok++;
      } catch (error) {
        console.error('Failed to delete meeting', id, error);
      }
    }
    setMeetings(meetings.filter((m: CurrentMeeting) => !selectedIds.has(m.id)));
    await refetchClients();
    if (currentMeeting && selectedIds.has(currentMeeting.id)) {
      setCurrentMeeting({ id: 'intro-call', title: '+ ' + t('newCall') });
      router.push('/');
    }
    if (ok > 0) {
      toast.success(t('deletedCount', { count: ok }), {
        description: t('dataRemoved'),
      });
    }
    if (ok < ids.length) {
      toast.error(t('failedToDeleteCount', { count: ids.length - ok }));
    }
    clearSelection();
    setBulkDeleteOpen(false);
  };

  // Handle modal editing of meeting names
  const handleEditStart = (meetingId: string, currentTitle: string) => {
    setEditModalState({
      isOpen: true,
      meetingId: meetingId,
      currentTitle: currentTitle
    });
    setEditingTitle(currentTitle);
  };

  const handleEditConfirm = async () => {
    const newTitle = editingTitle.trim();
    const meetingId = editModalState.meetingId;

    if (!meetingId) return;

    // Prevent empty titles
    if (!newTitle) {
      toast.error(t('titleEmptyError'));
      return;
    }

    try {
      await invoke('api_save_meeting_title', {
        meetingId: meetingId,
        title: newTitle,
      });

      // Update local state
      const updatedMeetings = meetings.map((m: CurrentMeeting) =>
        m.id === meetingId ? { ...m, title: newTitle } : m
      );
      setMeetings(updatedMeetings);

      // Update current meeting if it's the one being edited
      if (currentMeeting?.id === meetingId) {
        setCurrentMeeting({ id: meetingId, title: newTitle });
      }

      // Track the edit
      Analytics.trackButtonClick('edit_meeting_title', 'sidebar');

      toast.success(t('titleUpdatedSuccess'));

      // Close modal and reset state
      setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
      setEditingTitle('');
    } catch (error) {
      console.error('Failed to update meeting title:', error);
      toast.error(t('titleUpdateFailed'), {
        description: error instanceof Error ? error.message : String(error)
      });
    }
  };

  const handleEditCancel = () => {
    setEditModalState({ isOpen: false, meetingId: null, currentTitle: '' });
    setEditingTitle('');
  };

  const toggleFolder = (folderId: string) => {
    const newExpanded = new Set(expandedFolders);
    if (newExpanded.has(folderId)) {
      newExpanded.delete(folderId);
    } else {
      newExpanded.add(folderId);
    }
    setExpandedFolders(newExpanded);
    try {
      localStorage.setItem(EXPANDED_FOLDERS_KEY, JSON.stringify(Array.from(newExpanded)));
    } catch {
      /* remembering which folders were open is a convenience, not a requirement */
    }
  };

  /** Opens a folder, leaving it open if it already was. */
  const toggleFolderOpen = (folderId: string) => {
    if (!expandedFolders.has(folderId)) toggleFolder(folderId);
  };

  const handleClientDialogConfirm = async () => {
    if (!clientDialog) return;
    const name = clientDialog.name.trim();
    if (!name) {
      toast.error(t('clientNameEmptyError'));
      return;
    }

    try {
      if (clientDialog.mode === 'create') {
        const created = await invoke<{ id: string }>('api_create_client', { displayName: name });
        await refetchClients();
        // A brand-new folder is empty; opening it shows that it exists and is
        // waiting, instead of looking like nothing happened.
        toggleFolder(clientFolderId(created.id));
        toast.success(t('clientCreatedSuccess', { name }));
      } else if (clientDialog.clientId) {
        await invoke('api_rename_client', { clientId: clientDialog.clientId, displayName: name });
        await refetchClients();
        toast.success(t('clientRenamedSuccess'));
      }
      setClientDialog(null);
    } catch (error) {
      console.error('Failed to save client:', error);
      toast.error(t('clientSaveFailed'), {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleClientDelete = async () => {
    if (!clientDeleteState) return;
    try {
      await invoke('api_delete_client', { clientId: clientDeleteState.id });
      await Promise.all([refetchClients(), refetchMeetings()]);
      toast.success(t('clientDeletedSuccess'), {
        description: clientDeleteState.meetingCount > 0
          ? t('clientDeletedMeetingsKept', { count: clientDeleteState.meetingCount })
          : undefined,
      });
    } catch (error) {
      console.error('Failed to delete client:', error);
      toast.error(t('clientDeleteFailed'), {
        description: error instanceof Error ? error.message : String(error),
      });
    }
    setClientDeleteState(null);
  };


  // Expose setShowModelSettings to window for Rust tray to call
  useEffect(() => {
    (window as any).openSettings = () => {
      setShowModelSettings(true);
    };

    // Cleanup on unmount
    return () => {
      delete (window as any).openSettings;
    };
  }, []);

  const renderCollapsedIcons = () => {
    if (!isCollapsed) return null;

    const isMeetingPage = pathname?.includes('/meeting-details');
    const isSettingsPage = pathname === '/settings';

    return (
      <TooltipProvider>
        <div className="flex h-full flex-col items-center">
          <div className="flex flex-col items-center space-y-4 mt-4">
            <Logo isCollapsed={isCollapsed} />

            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={openGlobalSearch}
                  className="rounded-lg p-2 text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--af-bg)]"
                  aria-label={t('searchEverything')}
                >
                  <Search className="h-5 w-5" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">
                <p>{t('searchEverythingShortcut')}</p>
              </TooltipContent>
            </Tooltip>

            {/* New Recording */}
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={handleRecordingToggle}
                  disabled={isRecording}
                  className={`p-2 ${isRecording ? 'bg-red-500 cursor-not-allowed' : 'bg-red-500 hover:bg-red-600'} rounded-full transition-colors duration-150 shadow-sm`}
                >
                  {isRecording ? (
                    <Square className="w-5 h-5 text-white" />
                  ) : (
                    <Mic className="w-5 h-5 text-white" />
                  )}
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">
                <p>{isRecording ? t('recordingInProgressTooltip') : t('startRecording')}</p>
              </TooltipContent>
            </Tooltip>

            {/* Meetings */}
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={() => {
                    if (isCollapsed) toggleCollapse();
                  }}
                  className={`p-2 rounded-lg transition-colors duration-150 ${isMeetingPage ? 'bg-gray-100' : 'hover:bg-gray-100'
                    }`}
                >
                  <NotebookPen className="w-5 h-5 text-gray-600" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">
                <p>{t('meetingsTooltip')}</p>
              </TooltipContent>
            </Tooltip>

            {/* Import Audio (below Meetings) */}
            {betaFeatures.importAndRetranscribe && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    onClick={() => openImportDialog()}
                    className="p-2 rounded-lg transition-colors duration-150 hover:bg-blue-100 bg-blue-50"
                  >
                    <Upload className="w-5 h-5 text-blue-600" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="right">
                  <p>{t('importAudio')}</p>
                </TooltipContent>
              </Tooltip>
            )}
          </div>

          {/* Settings pinned to the bottom */}
          <div className="mt-auto mb-4">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={() => router.push('/settings')}
                  className={`p-2 rounded-lg transition-colors duration-150 ${isSettingsPage ? 'bg-gray-100' : 'hover:bg-gray-100'
                    }`}
                >
                  <Settings className="w-5 h-5 text-gray-600" />
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">
                <p>{tc('settings')}</p>
              </TooltipContent>
            </Tooltip>
          </div>
        </div>
      </TooltipProvider>
    );
  };

  const renderItem = (item: SidebarItem, depth = 0) => {
    const isExpanded = expandedFolders.has(item.id);
    // Meetings sit one step inside their folder; both stay tight to the left so
    // that as much of the title as possible survives the 256px sidebar.
    const paddingLeft = item.type === 'file' ? `${depth * 10 + 6}px` : `${depth * 10 + 4}px`;
    const isActive = item.type === 'file' && currentMeeting?.id === item.id;
    const isMeetingItem = item.id.includes('-') && !item.id.startsWith('intro-call');
    const isSelected = selectedIds.has(item.id);

    if (isCollapsed) return null;

    return (
      <div key={item.id}>
        <div
          className={`flex items-center transition-all duration-150 group select-none cursor-pointer ${item.type === 'folder'
            ? 'px-2 py-1.5 my-0.5 rounded-lg text-sm font-medium hover:bg-[var(--af-hover)]'
            : `px-2.5 py-2 my-0.5 rounded-lg text-sm ${isSelected ? 'bg-[var(--af-panel-2)] text-[var(--af-text)] ring-1 ring-[var(--af-accent)]/50' :
              isActive ? 'bg-[var(--af-panel-2)] text-[var(--af-text)] font-medium' :
                'hover:bg-[var(--af-hover)]'
            }`
            }`}
          style={{ paddingLeft }}
          onClick={(e) => {
            if (item.type === 'folder') {
              toggleFolder(item.id);
              return;
            }
            // Shift / Ctrl / Cmd click manages a multi-selection instead of
            // navigating, so several meetings can be deleted at once.
            if (isMeetingItem && (e.shiftKey || e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              handleMeetingSelect(item.id, e);
              return;
            }
            if (selectedIds.size > 0) clearSelection();
            setCurrentMeeting({ id: item.id, title: item.title });
            const basePath = item.id.startsWith('intro-call') ? '/' :
              item.id.includes('-') ? `/meeting-details?id=${item.id}` : `/notes/${item.id}`;
            router.push(basePath);
          }}
        >
          {item.type === 'folder' ? (
            <div className="relative flex w-full min-w-0 items-center gap-1.5">
              <span className="shrink-0 text-[var(--af-text-3)]">
                {isExpanded ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
              </span>
              <span className="shrink-0 text-[var(--af-text-3)]">
                {item.id === UNASSIGNED_FOLDER_ID ? <Inbox className="h-3.5 w-3.5" /> : <User className="h-3.5 w-3.5" />}
              </span>
              <span className="min-w-0 flex-1 truncate" title={item.title}>{item.title}</span>
              <span className="shrink-0 tabular-nums text-[11px] text-[var(--af-text-3)] group-hover:opacity-0">
                {item.meetingCount ?? 0}
              </span>

              {item.clientId && (
                <div className="absolute right-0 top-1/2 flex -translate-y-1/2 items-center gap-0.5 rounded-md bg-[var(--af-panel)] p-0.5 opacity-0 shadow-sm transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100">
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setClientDialog({ mode: 'rename', clientId: item.clientId!, name: item.title });
                    }}
                    className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-[var(--af-hover)] hover:text-[var(--af-accent)]"
                    aria-label={t('renameClientAria')}
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setClientDeleteState({
                        id: item.clientId!,
                        name: item.title,
                        meetingCount: item.meetingCount ?? 0,
                      });
                    }}
                    className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-red-500/10 hover:text-red-500"
                    aria-label={t('deleteClientAria')}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
              )}
            </div>
          ) : (
            (() => {
              const meetingDate = isMeetingItem ? parseMeetingDate(item) : null;
              const durationLabel = isMeetingItem ? formatDurationShort(item.durationSeconds) : '';
              return (
                <div className="relative flex w-full min-w-0 items-start gap-1.5">
                  {isMeetingItem ? (
                    <span className={`mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center ${isActive ? 'text-[var(--af-accent)]' : 'text-[var(--af-text-3)]'}`}>
                      {isActive ? <AudioLines className="h-3.5 w-3.5" /> : <FileText className="h-3.5 w-3.5" />}
                    </span>
                  ) : (
                    <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded bg-blue-100 text-blue-600">
                      <Plus className="h-3 w-3" />
                    </span>
                  )}

                  <div className="min-w-0 flex-1">
                    <div
                      className="truncate text-[13px] leading-snug"
                      title={item.title}
                    >
                      {item.title}
                    </div>
                    {isMeetingItem && (
                      <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 text-[11px] leading-tight text-[var(--af-text-3)]">
                        {meetingDate && <span className="truncate">{formatMeetingDate(meetingDate)}</span>}
                        {meetingDate && durationLabel && <span aria-hidden>·</span>}
                        {durationLabel && <span className="shrink-0 tabular-nums">{durationLabel}</span>}
                      </div>
                    )}
                  </div>

                  {isMeetingItem && (
                    <div className="absolute right-0 top-1/2 flex -translate-y-1/2 items-center gap-0.5 rounded-md bg-[var(--af-panel)] p-0.5 opacity-0 shadow-sm transition-opacity duration-150 group-hover:opacity-100 group-focus-within:opacity-100">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          setAssignDialogMeetingId(item.id);
                        }}
                        className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-[var(--af-hover)] hover:text-[var(--af-accent)]"
                        aria-label={t('moveToClientAria')}
                      >
                        <FolderInput className="h-3.5 w-3.5" />
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleEditStart(item.id, item.title);
                        }}
                        className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-[var(--af-hover)] hover:text-[var(--af-accent)]"
                        aria-label={t('editMeetingTitleAria')}
                      >
                        <Pencil className="h-3.5 w-3.5" />
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleteModalState({ isOpen: true, itemId: item.id });
                        }}
                        className="rounded-md p-1 text-[var(--af-text-3)] hover:bg-red-500/10 hover:text-red-500"
                        aria-label={t('deleteMeetingAria')}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  )}
                </div>
              );
            })()
          )}
        </div>
        {item.type === 'folder' && isExpanded && item.children && (() => {
          // Only the unassigned folder is capped: it is the one that keeps
          // growing. A client folder is a history, and cutting it off at eight
          // would hide exactly the older sessions worth looking back at.
          const capped = item.id === UNASSIGNED_FOLDER_ID && !showAllMeetings;
          const shown = capped ? item.children.slice(0, RECENT_LIMIT) : item.children;
          const hasMore = item.id === UNASSIGNED_FOLDER_ID && item.children.length > RECENT_LIMIT;
          return (
            <div>
              {shown.map(child => renderItem(child, depth + 1))}
              {item.children.length === 0 && (
                <div
                  className="px-2 py-1.5 text-[11px] italic text-[var(--af-text-3)]"
                  style={{ paddingLeft: `${(depth + 1) * 10 + 6}px` }}
                >
                  {item.id === UNASSIGNED_FOLDER_ID ? t('noUnassignedMeetings') : t('clientHasNoMeetings')}
                </div>
              )}
              {hasMore && (
                <button
                  onClick={() => setShowAllMeetings(value => !value)}
                  className="mt-1 mb-2 flex w-full items-center justify-center gap-2 rounded-lg border border-[var(--af-border-strong)] px-3 py-1.5 text-xs font-medium text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)]"
                >
                  {showAllMeetings ? t('showRecentOnly') : t('viewAllLibrary')}
                  <ArrowRight className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          );
        })()}
      </div>
    );
  };

  return (
    <div className="fixed top-0 left-0 h-screen z-40">
      <ResizeHandle
        label={t('resizeSidebar')}
        valueNow={isCollapsed ? SIDEBAR_COLLAPSED_WIDTH : panelLayout.sidebarWidth}
        valueMin={SIDEBAR_COLLAPSED_WIDTH}
        valueMax={SIDEBAR_MAX_WIDTH}
        // The sidebar starts at the window's edge, so the pointer is its width.
        onDrag={(x) => dragSidebarTo(x, false)}
        onDragEnd={(x) => dragSidebarTo(x, true)}
        onStep={(direction, big) => {
          if (isCollapsed) {
            if (direction > 0) collapseSidebar(false);
            return;
          }
          const width = panelLayout.sidebarWidth + direction * (big ? 64 : 16);
          if (width < SIDEBAR_MIN_WIDTH) collapseSidebar(true);
          else setSidebarWidth(width);
        }}
        onReset={() => setSidebarWidth(SIDEBAR_DEFAULT_WIDTH)}
        className="absolute top-0 -right-1 h-full"
      />

      <div
        className="h-screen bg-white border-r shadow-sm flex flex-col transition-[width] duration-300"
        style={{ width: 'var(--sidebar-offset)' }}
      >
        {/* Header: brand, search, New Recording */}
        <div className="flex-shrink-0">
          {!isCollapsed && (
            <div className="px-3 pt-5 pb-4 space-y-4">
              <div data-tauri-drag-region="deep" className="pt-1 pb-1">
                <Logo isCollapsed={isCollapsed} />
              </div>

              <button
                onClick={openGlobalSearch}
                className="flex h-9 w-full items-center gap-2 rounded-lg border border-[var(--af-border)] bg-[var(--af-panel)] px-3 text-left text-sm text-[var(--af-text-3)] shadow-sm hover:border-[var(--af-border-strong)] hover:bg-[var(--af-panel-2)] hover:text-[var(--af-text-2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--af-bg)]"
              >
                <Search className="h-4 w-4 shrink-0" />
                <span className="min-w-0 flex-1 truncate">{t('searchEverything')}</span>
                <kbd className="shrink-0 rounded border border-[var(--af-border-strong)] bg-[var(--af-panel-2)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--af-text-3)]">Ctrl K</kbd>
              </button>

              <button
                onClick={handleRecordingToggle}
                disabled={isRecording}
                className={`w-full flex items-center justify-center gap-2 rounded-lg px-3 py-2.5 text-sm font-semibold text-[var(--af-accent-contrast)] transition-[filter] bg-[var(--af-accent)] ${isRecording ? 'opacity-70 cursor-not-allowed' : 'hover:brightness-110'}`}
              >
                {isRecording ? (
                  <>
                    <Square className="w-4 h-4" />
                    <span>{t('recordingInProgress')}</span>
                  </>
                ) : (
                  <>
                    <AudioLines className="w-4 h-4" />
                    <span>{t('newRecording')}</span>
                  </>
                )}
              </button>
            </div>
          )}
        </div>

        {/* Main content - scrollable area */}
        <div className="flex-1 flex flex-col min-h-0">
          {/* Content area */}
          <div className="flex-1 flex flex-col min-h-0">
            {renderCollapsedIcons()}
            {/* Library header: the section label plus "add a client" */}
            {!isCollapsed && (
              <div className="flex-shrink-0 flex items-center gap-2 px-4 pt-5 pb-2 text-xs font-semibold uppercase tracking-wider text-[var(--af-text-3)]">
                <Users className="h-3.5 w-3.5" />
                <span className="min-w-0 flex-1 truncate">{t('library')}</span>
                <button
                  onClick={() => setClientDialog({ mode: 'create', clientId: null, name: '' })}
                  className="rounded-md p-1 text-[var(--af-text-3)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-accent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)]"
                  aria-label={t('newClientAria')}
                  title={t('newClient')}
                >
                  <Plus className="h-3.5 w-3.5" />
                </button>
              </div>
            )}

            {/* Bulk-selection action bar */}
            {!isCollapsed && selectedIds.size > 0 && (
              <div className="mx-3 mb-1 flex items-center justify-between rounded-md bg-blue-50 px-3 py-2 text-sm">
                <span className="font-medium text-blue-700">{t('selectedCount', { count: selectedIds.size })}</span>
                <div className="flex items-center gap-2">
                  <button onClick={clearSelection} className="text-gray-500 hover:text-gray-700">{tc('clear')}</button>
                  <button
                    onClick={() => setBulkDeleteOpen(true)}
                    className="inline-flex items-center gap-1 rounded-md bg-red-500 px-2 py-1 font-medium text-white hover:bg-red-600"
                  >
                    <Trash2 className="w-3.5 h-3.5" /> {tc('delete')}
                  </button>
                </div>
              </div>
            )}

            {/* Scrollable library tree */}
            {!isCollapsed && (
              <div className="flex-1 overflow-y-auto custom-scrollbar min-h-0 px-2">
                {sidebarItems.map(item => renderItem(item, 0))}
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        {!isCollapsed && (
          <div className="flex-shrink-0 p-2 border-t border-[var(--af-border)]">
            {betaFeatures.importAndRetranscribe && (
              <button
                onClick={() => openImportDialog()}
                className="w-full flex items-center gap-2.5 px-3 py-2 mb-1 text-sm font-medium text-[var(--af-text-2)] hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] rounded-lg transition-colors"
              >
                <Upload className="w-4 h-4" />
                <span>{t('importAudio')}</span>
              </button>
            )}
            <button
              onClick={() => router.push('/settings')}
              className="w-full flex items-center gap-2.5 px-3 py-2 text-sm font-medium text-[var(--af-text-2)] hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] rounded-lg transition-colors"
            >
              <Settings className="w-4 h-4" />
              <span>{tc('settings')}</span>
            </button>
          </div>
        )}
      </div>

      {/* Confirmation Modal for Delete */}
      <ConfirmationModal
        isOpen={deleteModalState.isOpen}
        text={t('confirmDeleteMeeting')}
        onConfirm={handleDeleteConfirm}
        onCancel={() => setDeleteModalState({ isOpen: false, itemId: null })}
      />

      {/* Confirmation Modal for Bulk Delete */}
      <ConfirmationModal
        isOpen={bulkDeleteOpen}
        text={t('confirmBulkDelete', { count: selectedIds.size })}
        onConfirm={handleBulkDelete}
        onCancel={() => setBulkDeleteOpen(false)}
      />

      {/* Deleting a client never deletes recordings — say so on the button. */}
      <ConfirmationModal
        isOpen={clientDeleteState !== null}
        text={clientDeleteState && clientDeleteState.meetingCount > 0
          ? t('confirmDeleteClientWithMeetings', {
            name: clientDeleteState.name,
            count: clientDeleteState.meetingCount,
          })
          : t('confirmDeleteClient', { name: clientDeleteState?.name ?? '' })}
        onConfirm={handleClientDelete}
        onCancel={() => setClientDeleteState(null)}
      />

      {/* New client / rename client */}
      <Dialog open={clientDialog !== null} onOpenChange={(open) => { if (!open) setClientDialog(null); }}>
        <DialogContent className="sm:max-w-[425px]">
          <VisuallyHidden>
            <DialogTitle>{clientDialog?.mode === 'rename' ? t('renameClient') : t('newClient')}</DialogTitle>
          </VisuallyHidden>
          <div className="py-4">
            <h3 className="text-lg font-semibold mb-4">
              {clientDialog?.mode === 'rename' ? t('renameClient') : t('newClient')}
            </h3>
            <label htmlFor="client-name" className="mb-2 block text-sm font-medium text-[var(--af-text-2)]">
              {t('clientNameLabel')}
            </label>
            <input
              id="client-name"
              type="text"
              value={clientDialog?.name ?? ''}
              onChange={(e) => setClientDialog(state => (state ? { ...state, name: e.target.value } : state))}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleClientDialogConfirm();
                else if (e.key === 'Escape') setClientDialog(null);
              }}
              className="w-full rounded-md border border-[var(--af-border-strong)] bg-[var(--af-panel)] px-3 py-2 text-[var(--af-text)] focus:border-transparent focus:outline-none focus:ring-2 focus:ring-[var(--af-accent)]"
              placeholder={t('clientNamePlaceholder')}
              autoFocus
            />
            {/* Same name, different person, is legal — this warns, never blocks. */}
            {duplicateClientName && (
              <p className="mt-2 text-xs text-amber-500">{t('clientNameAlreadyUsed')}</p>
            )}
          </div>
          <DialogFooter>
            <button
              onClick={() => setClientDialog(null)}
              className="rounded-md bg-[var(--af-panel-2)] px-4 py-2 text-sm font-medium text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)]"
            >
              {tc('cancel')}
            </button>
            <button
              onClick={handleClientDialogConfirm}
              className="rounded-md bg-[var(--af-accent)] px-4 py-2 text-sm font-medium text-[var(--af-accent-contrast)] transition-[filter] hover:brightness-110"
            >
              {tc('save')}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Filing one recording under a client */}
      <ClientPickerDialog
        open={assignDialogMeetingId !== null}
        meetingId={assignDialogMeetingId}
        currentClientId={meetings.find(m => m.id === assignDialogMeetingId)?.client_id ?? null}
        onOpenChange={(open) => { if (!open) setAssignDialogMeetingId(null); }}
        onMoved={(clientId) => { if (clientId) toggleFolderOpen(clientFolderId(clientId)); }}
      />

      {/* Edit Meeting Title Modal */}
      <Dialog open={editModalState.isOpen} onOpenChange={(open) => {
        if (!open) handleEditCancel();
      }}>
        <DialogContent className="sm:max-w-[425px]">
          <VisuallyHidden>
            <DialogTitle>{t('editMeetingTitle')}</DialogTitle>
          </VisuallyHidden>
          <div className="py-4">
            <h3 className="text-lg font-semibold mb-4">{t('editMeetingTitle')}</h3>
            <div className="space-y-4">
              <div>
                <label htmlFor="meeting-title" className="block text-sm font-medium text-gray-700 mb-2">
                  {t('meetingTitleLabel')}
                </label>
                <input
                  id="meeting-title"
                  type="text"
                  value={editingTitle}
                  onChange={(e) => setEditingTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      handleEditConfirm();
                    } else if (e.key === 'Escape') {
                      handleEditCancel();
                    }
                  }}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  placeholder={t('meetingTitlePlaceholder')}
                  autoFocus
                />
              </div>
            </div>
          </div>
          <DialogFooter>
            <button
              onClick={handleEditCancel}
              className="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-md transition-colors"
            >
              {tc('cancel')}
            </button>
            <button
              onClick={handleEditConfirm}
              className="px-4 py-2 text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 rounded-md transition-colors"
            >
              {tc('save')}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
};

export default Sidebar;
