import { useCallback, useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { AlertCircle, CheckCircle2, Download, Loader2, Trash2, Languages } from 'lucide-react';
import { Button } from './ui/button';
import { Progress } from './ui/progress';

export const GIGAAM_MODEL_NAME = 'gigaam-v3-e2e-rnnt-int8';

export interface GigaamModelStatus {
    name: string;
    installed: boolean;
    partial: boolean;
    downloading: boolean;
    progress: number;
    loaded: boolean;
    sizeMb: number;
    path: string;
}

interface DownloadProgressPayload {
    modelName: string;
    progress: {
        downloadedMb: number;
        totalMb: number;
        speedMbps: number;
        percent: number;
    };
}

interface GigaamModelManagerProps {
    isSelected: boolean;
    disabled?: boolean;
    onSelect: (modelName: string) => void | Promise<unknown>;
    /** The one model this list recommends; the card then says so instead of its own label. */
    recommended?: boolean;
}

export function GigaamModelManager({ isSelected, disabled, onSelect, recommended }: GigaamModelManagerProps) {
    const t = useTranslations('settings');
    const [status, setStatus] = useState<GigaamModelStatus | null>(null);
    const [percent, setPercent] = useState(0);
    const [speed, setSpeed] = useState<number | null>(null);
    const [isDownloading, setIsDownloading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const refreshStatus = useCallback(async () => {
        try {
            const next = await invoke<GigaamModelStatus>('gigaam_get_model_status');
            setStatus(next);
            setIsDownloading(next.downloading);
            if (!next.downloading) setPercent(next.installed ? 100 : 0);
        } catch (statusError) {
            console.error('Failed to read GigaAM model status:', statusError);
            setError(t('gigaamStatusFailed'));
        }
    }, [t]);

    useEffect(() => {
        void refreshStatus();
    }, [refreshStatus]);

    useEffect(() => {
        const listeners: Promise<UnlistenFn>[] = [
            listen<DownloadProgressPayload>('gigaam-model-download-progress', (event) => {
                setPercent(event.payload.progress.percent);
                setSpeed(event.payload.progress.speedMbps);
            }),
            listen('gigaam-model-download-complete', () => {
                setIsDownloading(false);
                setSpeed(null);
                void refreshStatus();
            }),
            listen<{ error?: string }>('gigaam-model-download-failed', (event) => {
                setIsDownloading(false);
                setSpeed(null);
                setError(event.payload?.error || t('gigaamDownloadFailed'));
                void refreshStatus();
            }),
            listen('gigaam-model-download-cancelled', () => {
                setIsDownloading(false);
                setSpeed(null);
                void refreshStatus();
            }),
        ];

        return () => {
            listeners.forEach((pending) => {
                void pending.then((unlisten) => unlisten());
            });
        };
    }, [refreshStatus, t]);

    const startDownload = async () => {
        setError(null);
        setIsDownloading(true);
        setPercent(status?.partial ? percent : 0);
        try {
            await invoke('gigaam_download_model');
        } catch (downloadError) {
            // The failure event already carries the message; ignore the rejection
            console.error('GigaAM download failed:', downloadError);
        }
    };

    const cancelDownload = async () => {
        try {
            await invoke('gigaam_cancel_download');
        } catch (cancelError) {
            console.error('Failed to cancel the GigaAM download:', cancelError);
        }
    };

    const deleteModel = async () => {
        setError(null);
        try {
            await invoke('gigaam_delete_model');
            await refreshStatus();
        } catch (deleteError) {
            setError(typeof deleteError === 'string' ? deleteError : String(deleteError));
        }
    };

    const installed = status?.installed ?? false;
    const canSelect = installed && !disabled && !isSelected && !isDownloading;

    return (
        <div
            className={`space-y-4 rounded-xl border p-4 transition-colors ${canSelect ? 'cursor-pointer hover:border-[var(--af-accent)]' : ''} ${isSelected
                ? 'border-[var(--af-accent)] bg-[var(--af-accent-soft)] ring-1 ring-blue-500/20'
                : 'border-[var(--af-border-strong)] bg-[var(--af-panel-2)]'}`}
            role={canSelect ? 'button' : undefined}
            tabIndex={canSelect ? 0 : undefined}
            aria-pressed={isSelected}
            onClick={() => {
                if (canSelect) void onSelect(GIGAAM_MODEL_NAME);
            }}
            onKeyDown={(event) => {
                if (canSelect && (event.key === 'Enter' || event.key === ' ')) {
                    event.preventDefault();
                    void onSelect(GIGAAM_MODEL_NAME);
                }
            }}
        >
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 items-start gap-3">
                    <Languages className="mt-0.5 h-5 w-5 shrink-0 text-sky-400" />
                    <div>
                        <div className="flex flex-wrap items-center gap-2">
                            <h4 className="font-semibold">GigaAM v3</h4>
                            {recommended ? (
                                <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-emerald-500">
                                    {t('badgeRecommendedLive')}
                                </span>
                            ) : (
                                <span className="rounded-full bg-sky-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-sky-400">
                                    {t('gigaamBadge')}
                                </span>
                            )}
                        </div>
                        <p className="mt-1 text-sm text-[var(--af-text-2)]">
                            {t('gigaamDescription')}
                        </p>
                    </div>
                </div>
                {isSelected ? (
                    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-blue-500/40 bg-blue-500/10 px-2.5 py-1 text-xs font-medium text-blue-400">
                        <CheckCircle2 className="h-3.5 w-3.5" /> {t('selectedForLive')}
                    </span>
                ) : installed ? (
                    <span className="rounded-full border border-[var(--af-border-strong)] px-2.5 py-1 text-xs font-medium text-[var(--af-text-2)]">
                        {t('clickToSelect')}
                    </span>
                ) : (
                    <span className="text-xs text-[var(--af-text-3)]">
                        {t('gigaamSizeHint', { size: status?.sizeMb ?? 215 })}
                    </span>
                )}
            </div>

            <div
                className="space-y-3"
                onClick={(event) => event.stopPropagation()}
                onKeyDown={(event) => event.stopPropagation()}
            >
                {isDownloading ? (
                    <>
                        <Progress value={percent} />
                        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--af-text-3)]">
                            <span>
                                {speed !== null
                                    ? t('gigaamDownloadingWithSpeed', {
                                        percent,
                                        speed: speed.toFixed(1),
                                    })
                                    : t('gigaamDownloading', { percent })}
                            </span>
                            <Button type="button" variant="outline" size="sm" onClick={() => void cancelDownload()}>
                                {t('gigaamCancel')}
                            </Button>
                        </div>
                    </>
                ) : installed ? (
                    <div className="flex flex-wrap items-center gap-2">
                        <span className="inline-flex items-center gap-1.5 text-xs text-emerald-500">
                            <CheckCircle2 className="h-3.5 w-3.5" />
                            {t('gigaamInstalled', { size: status?.sizeMb ?? 215 })}
                        </span>
                        <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={() => void deleteModel()}
                            disabled={isSelected}
                            title={isSelected ? t('gigaamDeleteBlocked') : undefined}
                        >
                            <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                            {t('gigaamDelete')}
                        </Button>
                    </div>
                ) : (
                    <Button type="button" variant="outline" className="w-full" onClick={() => void startDownload()}>
                        {status === null ? (
                            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        ) : (
                            <Download className="mr-2 h-4 w-4" />
                        )}
                        {status?.partial
                            ? t('gigaamResumeDownload')
                            : t('gigaamDownload', { size: status?.sizeMb ?? 215 })}
                    </Button>
                )}

                {error ? (
                    <p className="inline-flex items-start gap-1.5 text-sm text-red-400">
                        <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" /> {error}
                    </p>
                ) : null}
            </div>
        </div>
    );
}
