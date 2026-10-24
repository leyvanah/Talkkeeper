'use client'

/**
 * `ConfirmDialog` for the common case: one question, one action.
 *
 * The caller keeps a `ConfirmRequest | null` and passes it in; the dialog is
 * open while it is set. Pressing the confirm button runs `run` once (a double
 * click does not run it twice), keeps the dialog open with the button off
 * until it is done, then closes. `run` reports its own failures to the owner;
 * anything it lets through only reaches the console.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import { exclusive } from '@/lib/run-once'

export interface ConfirmRequest {
  title: string
  description: ReactNode
  confirmLabel: string
  busyLabel?: string
  /** Red confirm button; on unless the action loses nothing. */
  destructive?: boolean
  run: () => Promise<void> | void
}

export function ConfirmAction({
  request,
  onClose,
}: {
  request: ConfirmRequest | null
  onClose: () => void
}) {
  const [busy, setBusy] = useState(false)
  // The text stays while the dialog fades out after `request` is cleared.
  const [shown, setShown] = useState<ConfirmRequest | null>(request)
  useEffect(() => {
    if (request) setShown(request)
  }, [request])

  const closeRef = useRef(onClose)
  closeRef.current = onClose
  const [runOnce] = useState(() =>
    exclusive(async (run: ConfirmRequest['run']) => {
      setBusy(true)
      try {
        await run()
      } catch (error) {
        console.error('Confirmed action failed:', error)
      } finally {
        setBusy(false)
        closeRef.current()
      }
    }),
  )

  const current = request ?? shown
  if (!current) return null

  return (
    <ConfirmDialog
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={current.title}
      description={current.description}
      confirmLabel={current.confirmLabel}
      busyLabel={current.busyLabel}
      busy={busy}
      destructive={current.destructive ?? true}
      onConfirm={() => {
        if (request) void runOnce(request.run)
      }}
    />
  )
}
