'use client';

/**
 * "Filed under N" for the meeting header, as quiet text rather than a chip. Clicking it opens the same
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
        className={`inline-flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 transition-colors hover:bg-[var(--af-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--af-accent)] ${client
          ? 'text-[var(--af-text-2)] hover:text-[var(--af-text)]'
          : 'text-[var(--af-text-3)] hover:text-[var(--af-text-2)]'}`}
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
