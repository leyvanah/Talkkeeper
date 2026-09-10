'use client';

/**
 * Files one recording under a client.
 *
 * Used from the sidebar row and from the meeting header, so both places behave
 * the same way. The single text field both filters the list and names a new
 * client: filing a recording usually happens the first time that person is
 * recorded, and making that a second dialog would put a wall exactly where the
 * common case is.
 */

import React, { useMemo, useState } from 'react';
import { Inbox, Plus, User } from 'lucide-react';
import { useTranslations } from 'next-intl';
import { toast } from 'sonner';
import { invoke } from '@tauri-apps/api/core';

import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from '@/components/ui/dialog';
import { VisuallyHidden } from '@/components/ui/visually-hidden';

interface ClientPickerDialogProps {
  open: boolean;
  /**
   * The recording being filed. Null with `onSelect` set means nothing is saved
   * yet — the dialog only reports the choice, as it does before a recording.
   */
  meetingId: string | null;
  /** Client the recording is filed under right now, if any. */
  currentClientId?: string | null;
  onOpenChange: (open: boolean) => void;
  /** Called after the move succeeds, with the new client id (null = unassigned). */
  onMoved?: (clientId: string | null) => void;
  /** When set, the choice is handed over instead of written to a meeting. */
  onSelect?: (clientId: string | null) => void;
  /** Heading, when "move to a client" is the wrong words for the moment. */
  title?: string;
}

const normalize = (name: string) => name.trim().replace(/\s+/g, ' ').toLowerCase();

export const ClientPickerDialog: React.FC<ClientPickerDialogProps> = ({
  open,
  meetingId,
  currentClientId,
  onOpenChange,
  onMoved,
  onSelect,
  title,
}) => {
  const t = useTranslations('sidebar');
  const { clients, refetchClients, assignMeetingToClient } = useSidebar();
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);

  const trimmed = query.trim();
  const matches = useMemo(
    () => clients.filter(client => normalize(client.displayName).includes(normalize(query))),
    [clients, query]
  );
  const nameIsTaken = trimmed.length > 0 && clients.some(
    client => normalize(client.displayName) === normalize(trimmed)
  );

  const close = () => {
    setQuery('');
    onOpenChange(false);
  };

  const move = async (clientId: string | null, name?: string) => {
    if (busy) return;
    if (onSelect) {
      onSelect(clientId);
      close();
      return;
    }
    if (!meetingId) return;
    setBusy(true);
    try {
      await assignMeetingToClient(meetingId, clientId);
      toast.success(clientId ? t('meetingMovedToClient', { name: name ?? '' }) : t('meetingUnassigned'));
      onMoved?.(clientId);
      close();
    } catch (error) {
      console.error('Failed to change the meeting client:', error);
      toast.error(t('meetingMoveFailed'), {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusy(false);
    }
  };

  const createAndMove = async () => {
    if (!trimmed || busy || (!meetingId && !onSelect)) return;
    setBusy(true);
    try {
      const created = await invoke<{ id: string }>('api_create_client', { displayName: trimmed });
      await refetchClients();
      if (onSelect) {
        onSelect(created.id);
      } else {
        await assignMeetingToClient(meetingId!, created.id);
        toast.success(t('meetingMovedToClient', { name: trimmed }));
        onMoved?.(created.id);
      }
      close();
    } catch (error) {
      console.error('Failed to create the client:', error);
      toast.error(t('clientSaveFailed'), {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!next) close(); }}>
      <DialogContent className="sm:max-w-[425px]">
        <VisuallyHidden>
          <DialogTitle>{title ?? t('moveToClient')}</DialogTitle>
        </VisuallyHidden>
        <div className="py-4">
          <h3 className="mb-4 text-lg font-semibold text-[var(--af-text)]">{title ?? t('moveToClient')}</h3>

          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && trimmed && !nameIsTaken) createAndMove();
              else if (e.key === 'Escape') close();
            }}
            className="mb-2 w-full rounded-md border border-[var(--af-border-strong)] bg-[var(--af-panel)] px-3 py-2 text-sm text-[var(--af-text)] focus:border-transparent focus:outline-none focus:ring-2 focus:ring-[var(--af-accent)]"
            placeholder={t('findOrCreateClient')}
            autoFocus
          />

          <div className="max-h-64 space-y-1 overflow-y-auto custom-scrollbar">
            {matches.map(client => {
              const isCurrent = client.id === currentClientId;
              return (
                <button
                  key={client.id}
                  onClick={() => move(client.id, client.displayName)}
                  disabled={isCurrent || busy}
                  className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors ${isCurrent
                    ? 'bg-[var(--af-panel-2)] text-[var(--af-text-3)]'
                    : 'text-[var(--af-text)] hover:bg-[var(--af-hover)]'}`}
                >
                  <User className="h-3.5 w-3.5 shrink-0 text-[var(--af-text-3)]" />
                  <span className="min-w-0 flex-1 truncate">{client.displayName}</span>
                  {isCurrent && <span className="shrink-0 text-[11px]">{t('currentClient')}</span>}
                </button>
              );
            })}

            <button
              onClick={() => move(null)}
              disabled={!currentClientId || busy}
              className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors ${!currentClientId
                ? 'bg-[var(--af-panel-2)] text-[var(--af-text-3)]'
                : 'text-[var(--af-text-2)] hover:bg-[var(--af-hover)]'}`}
            >
              <Inbox className="h-3.5 w-3.5 shrink-0 text-[var(--af-text-3)]" />
              <span className="min-w-0 flex-1 truncate">{t('unassignedMeetings')}</span>
              {!currentClientId && <span className="shrink-0 text-[11px]">{t('currentClient')}</span>}
            </button>
          </div>

          {trimmed.length > 0 && !nameIsTaken && (
            <button
              onClick={createAndMove}
              disabled={busy}
              className="mt-3 flex w-full items-center justify-center gap-2 rounded-lg border border-[var(--af-border-strong)] px-3 py-2 text-sm font-medium text-[var(--af-text-2)] transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)]"
            >
              <Plus className="h-4 w-4 shrink-0" />
              <span className="min-w-0 truncate">{t('createClientAndMove', { name: trimmed })}</span>
            </button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
};

export default ClientPickerDialog;
