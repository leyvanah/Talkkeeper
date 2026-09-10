'use client';

/**
 * "Filed under N" chip for the meeting header. Clicking it opens the same
 * picker the sidebar row uses.
 *
 * A recording with no client says so instead of showing nothing: an empty spot
 * would look like the feature is missing, and this is the moment — the meeting
 * open on screen — when the owner knows who it was with.
 */

import React, { useState } from 'react';
import { User, UserPlus } from 'lucide-react';
import { useTranslations } from 'next-intl';

import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { ClientPickerDialog } from '@/components/ClientPickerDialog';

export const MeetingClientBadge: React.FC<{ meetingId?: string }> = ({ meetingId }) => {
  const t = useTranslations('sidebar');
  const { meetings, clients } = useSidebar();
  const [pickerOpen, setPickerOpen] = useState(false);

  if (!meetingId) return null;

  const clientId = meetings.find(meeting => meeting.id === meetingId)?.client_id ?? null;
  const client = clientId ? clients.find(candidate => candidate.id === clientId) : undefined;

  return (
    <>
      <button
        onClick={() => setPickerOpen(true)}
        className={`inline-flex min-w-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 transition-colors ${client
          ? 'border-[var(--af-border-strong)] bg-[var(--af-panel-2)] text-[var(--af-text-2)] hover:border-[var(--af-accent)] hover:text-[var(--af-text)]'
          : 'border-dashed border-[var(--af-border-strong)] text-[var(--af-text-3)] hover:border-[var(--af-accent)] hover:text-[var(--af-text-2)]'}`}
        title={t('moveToClient')}
      >
        {client
          ? <User size={14} className="shrink-0 text-[var(--af-text-3)]" />
          : <UserPlus size={14} className="shrink-0" />}
        <span className="truncate">{client ? client.displayName : t('noClientYet')}</span>
      </button>

      <ClientPickerDialog
        open={pickerOpen}
        meetingId={meetingId}
        currentClientId={clientId}
        onOpenChange={setPickerOpen}
      />
    </>
  );
};

export default MeetingClientBadge;
