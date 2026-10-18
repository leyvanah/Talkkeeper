'use client'

/**
 * Asks before something that cannot be taken back.
 *
 * Built on the shared `Dialog`, so Esc and the close button work and focus
 * stays inside. Focus lands on "Cancel", or on the first field when the dialog
 * asks for something (a password); Enter in that field confirms. The confirm
 * button is off while the action runs and a second press is ignored, and the
 * dialog does not close under a running action.
 */

import { useRef, type ReactNode } from 'react'
import { useTranslations } from 'next-intl'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  busyLabel,
  busy = false,
  confirmDisabled = false,
  destructive = false,
  onConfirm,
  children,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: ReactNode
  confirmLabel: string
  /** Shown on the confirm button while the action runs. */
  busyLabel?: string
  busy?: boolean
  confirmDisabled?: boolean
  destructive?: boolean
  onConfirm: () => void
  /** Anything the action needs typed in, between the text and the buttons. */
  children?: ReactNode
}) {
  const t = useTranslations('common')
  const cancel = useRef<HTMLButtonElement>(null)

  const confirm = () => {
    if (busy || confirmDisabled) return
    onConfirm()
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (busy && !next) return
        onOpenChange(next)
      }}
    >
      <DialogContent
        className="sm:max-w-md"
        onOpenAutoFocus={(event) => {
          // With a field to fill, the default (the first field) is right.
          if (children) return
          event.preventDefault()
          cancel.current?.focus()
        }}
      >
        <form
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault()
            confirm()
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          {children}
          <DialogFooter className="gap-2 sm:gap-0">
            <Button
              ref={cancel}
              type="button"
              variant="outline"
              disabled={busy}
              onClick={() => onOpenChange(false)}
            >
              {t('cancel')}
            </Button>
            <Button
              type="submit"
              variant={destructive ? 'destructive' : 'default'}
              disabled={busy || confirmDisabled}
            >
              {busy && busyLabel ? busyLabel : confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
