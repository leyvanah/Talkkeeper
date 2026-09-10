'use client';

/**
 * Chooses who the recording about to start is with.
 *
 * The alternative — file every recording afterwards from the tree — puts the
 * work at the one moment the owner least wants it: right after a session ends.
 * Choosing beforehand costs one click while nothing is happening yet.
 *
 * Nothing here blocks recording. With no choice made the recording still runs
 * and lands in "Unassigned", exactly as it did before clients existed.
 */

import React, { useEffect, useState } from 'react';
import { User, UserPlus } from 'lucide-react';
import { useTranslations } from 'next-intl';

import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { ClientPickerDialog } from '@/components/ClientPickerDialog';
import { getPendingRecordingClient, setPendingRecordingClient } from '@/lib/recording-client';

export const RecordingClientSelector: React.FC = () => {
  const t = useTranslations('sidebar');
  const { clients } = useSidebar();
  const [clientId, setClientId] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  // sessionStorage is not there during the static export's prerender.
  useEffect(() => {
    setClientId(getPendingRecordingClient());
  }, []);

  // A client deleted between the choice and the recording must not leave a
  // dangling id behind: the backend would refuse it and the recording would
  // quietly end up unassigned anyway, without saying so here.
  useEffect(() => {
    if (clientId && clients.length > 0 && !clients.some(client => client.id === clientId)) {
      setPendingRecordingClient(null);
      setClientId(null);
    }
  }, [clientId, clients]);

  const client = clientId ? clients.find(candidate => candidate.id === clientId) : undefined;

  return (
    <>
      <button
        onClick={() => setPickerOpen(true)}
        className={`inline-flex min-w-0 max-w-[220px] items-center gap-1.5 rounded-full border px-3 py-1 text-sm transition-colors ${client
          ? 'border-[var(--af-border-strong)] bg-[var(--af-panel-2)] text-[var(--af-text-2)] hover:border-[var(--af-accent)] hover:text-[var(--af-text)]'
          : 'border-dashed border-[var(--af-border-strong)] text-[var(--af-text-3)] hover:border-[var(--af-accent)] hover:text-[var(--af-text-2)]'}`}
      >
        {client
          ? <User size={14} className="shrink-0 text-[var(--af-text-3)]" />
          : <UserPlus size={14} className="shrink-0" />}
        <span className="truncate">{client ? client.displayName : t('recordingForClient')}</span>
      </button>

      <ClientPickerDialog
        open={pickerOpen}
        meetingId={null}
        currentClientId={clientId}
        title={t('recordingForClient')}
        onOpenChange={setPickerOpen}
        onSelect={(selected) => {
          setPendingRecordingClient(selected);
          setClientId(selected);
        }}
      />
    </>
  );
};

export default RecordingClientSelector;
