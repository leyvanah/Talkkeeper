/**
 * Recordings whose transcript never reached the database — a crash, a forced
 * quit — as the recovery dialog shows them.
 *
 * The transcript itself lives in the backend's per-recording journal
 * (`transcript.journal`, sealed line by line with the archive key). Nothing is
 * stored in the window: this module only reshapes what the backend returns.
 */

/** One unsaved recording, as returned by `list_unsaved_recordings`. */
export interface UnsavedRecording {
  folderPath: string
  title: string
  /** RFC 3339. */
  startedAt: string
  /** Milliseconds since the epoch. */
  lastUpdated: number
  segments: JournalSegment[]
}

/** A line of transcript from the journal (the backend's TranscriptSegment). */
export interface JournalSegment {
  id: string
  text: string
  audio_start_time: number
  audio_end_time: number
  duration: number
  display_time: string
  confidence: number
  sequence_id: number
  speaker?: string
}

/** What the recovery dialog lists. `meetingId` is the recording's folder. */
export interface MeetingMetadata {
  meetingId: string
  title: string
  startTime: number
  lastUpdated: number
  transcriptCount: number
  /** Set only when there is audio to recover alongside the transcript. */
  folderPath?: string
}

/** A line of transcript as the recovery dialog previews it. */
export interface StoredTranscript {
  id: string
  text: string
  timestamp: string
  confidence: number
  sequenceId: number
  audio_start_time?: number
  audio_end_time?: number
  duration?: number
  speaker?: string
}

export function toMeetingMetadata(recording: UnsavedRecording): MeetingMetadata {
  const started = Date.parse(recording.startedAt)
  return {
    meetingId: recording.folderPath,
    title: recording.title,
    startTime: Number.isNaN(started) ? recording.lastUpdated : started,
    lastUpdated: recording.lastUpdated,
    transcriptCount: recording.segments.length,
    folderPath: recording.folderPath,
  }
}

export function toStoredTranscripts(recording: UnsavedRecording): StoredTranscript[] {
  return recording.segments.map((segment) => ({
    id: segment.id,
    text: segment.text,
    timestamp: segment.display_time,
    confidence: segment.confidence,
    sequenceId: segment.sequence_id,
    audio_start_time: segment.audio_start_time,
    audio_end_time: segment.audio_end_time,
    duration: segment.duration,
    speaker: segment.speaker,
  }))
}

/**
 * The name of the window's old recovery store. It held every line of every
 * recording in the clear; it is deleted on startup and never written again.
 */
export const LEGACY_RECOVERY_DB = 'MeetilyRecoveryDB'

/** Deletes the old IndexedDB recovery store, if this profile still has it. */
export function deleteLegacyRecoveryStore(): Promise<void> {
  return new Promise((resolve) => {
    try {
      if (typeof indexedDB === 'undefined') return resolve()
      const request = indexedDB.deleteDatabase(LEGACY_RECOVERY_DB)
      request.onsuccess = () => resolve()
      request.onerror = () => resolve()
      // Another tab holding it open: the delete completes once it closes.
      request.onblocked = () => resolve()
    } catch {
      resolve()
    }
  })
}
