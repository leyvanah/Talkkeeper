import { useCallback, useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, CheckCircle2, Loader2, Server } from 'lucide-react';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Label } from './ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { logFailure } from '@/lib/failure';

export interface ExternalSttConfig {
    url: string;
    mode: 'multipart' | 'raw';
    fileField: string;
    model: string | null;
    languageField: string;
    sendLanguage: boolean;
    responsePath: string;
    timeoutMs: number;
    maxRetries: number;
    authToken: string | null;
}

interface ExternalSttTestResult {
    ok: boolean;
    latencyMs: number;
    text: string;
}

export const DEFAULT_EXTERNAL_STT_CONFIG: ExternalSttConfig = {
    url: '',
    mode: 'multipart',
    fileField: 'file',
    model: null,
    languageField: 'language',
    sendLanguage: true,
    responsePath: 'text',
    timeoutMs: 30000,
    maxRetries: 2,
    authToken: null,
};

/** Host:port of the endpoint - the label shown as the "model" for this provider. */
export function externalSttLabel(config: ExternalSttConfig): string {
    const host = config.url.trim().replace(/^https?:\/\//, '').split('/')[0];
    return config.model?.trim() || host || 'external';
}

interface ExternalSttSettingsProps {
    isSelected: boolean;
    disabled?: boolean;
    onSelect: (modelLabel: string) => void | Promise<unknown>;
}

export function ExternalSttSettings({ isSelected, disabled, onSelect }: ExternalSttSettingsProps) {
    const t = useTranslations('settings');
    const [config, setConfig] = useState<ExternalSttConfig>(DEFAULT_EXTERNAL_STT_CONFIG);
    const [savedUrl, setSavedUrl] = useState('');
    const [isSaving, setIsSaving] = useState(false);
    const [isTesting, setIsTesting] = useState(false);
    const [saved, setSaved] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [testMessage, setTestMessage] = useState<string | null>(null);

    useEffect(() => {
        invoke<ExternalSttConfig>('api_get_external_stt_config')
            .then((loaded) => {
                setConfig({ ...DEFAULT_EXTERNAL_STT_CONFIG, ...loaded });
                setSavedUrl(loaded.url || '');
            })
            .catch((loadError) => {
                console.error('Failed to load external STT config:', loadError);
                setError(t('externalSttLoadFailed'));
            });
    }, []);

    const update = useCallback(<K extends keyof ExternalSttConfig>(key: K, value: ExternalSttConfig[K]) => {
        setConfig((current) => ({ ...current, [key]: value }));
        setSaved(false);
        setTestMessage(null);
        setError(null);
    }, []);


    const save = async () => {
        setIsSaving(true);
        setError(null);
        setTestMessage(null);
        try {
            await invoke('api_save_external_stt_config', { config });
            setSavedUrl(config.url.trim());
            setSaved(true);
            window.setTimeout(() => setSaved(false), 2000);
            // Keep the stored live model label in step with the endpoint
            if (isSelected) {
                await onSelect(externalSttLabel(config));
            }
        } catch (saveError) {
            logFailure('external-stt-save', saveError);
            setError(t('externalSttSaveFailed'));
        } finally {
            setIsSaving(false);
        }
    };

    const testConnection = async () => {
        setIsTesting(true);
        setError(null);
        setTestMessage(null);
        try {
            const result = await invoke<ExternalSttTestResult>('api_test_external_stt', { config });
            setTestMessage(t('externalSttTestOk', { ms: result.latencyMs }));
        } catch (testError) {
            logFailure('external-stt-test', testError);
            setError(t('externalSttTestFailed'));
        } finally {
            setIsTesting(false);
        }
    };

    const isConfigured = savedUrl.trim().length > 0;
    const canSelect = isConfigured && !disabled && !isSelected;

    return (
        <div
            className={`space-y-4 rounded-xl border p-4 transition-colors ${canSelect ? 'cursor-pointer hover:border-[var(--af-accent)]' : ''} ${isSelected
                ? 'border-[var(--af-accent)] bg-[var(--af-accent-soft)] ring-1 ring-blue-500/20'
                : 'border-[var(--af-border-strong)] bg-[var(--af-panel-2)]'}`}
            role={canSelect ? 'button' : undefined}
            tabIndex={canSelect ? 0 : undefined}
            aria-pressed={isSelected}
            onClick={() => {
                if (canSelect) void onSelect(externalSttLabel(config));
            }}
            onKeyDown={(event) => {
                if (canSelect && (event.key === 'Enter' || event.key === ' ')) {
                    event.preventDefault();
                    void onSelect(externalSttLabel(config));
                }
            }}
        >
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 items-start gap-3">
                    <Server className="mt-0.5 h-5 w-5 shrink-0 text-emerald-400" />
                    <div>
                        <div className="flex flex-wrap items-center gap-2">
                            <h4 className="font-semibold">{t('externalSttTitle')}</h4>
                            <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-emerald-500">
                                {t('externalSttBadge')}
                            </span>
                        </div>
                        <p className="mt-1 text-sm text-[var(--af-text-2)]">
                            {t('externalSttDescription')}
                        </p>
                    </div>
                </div>
                {isSelected ? (
                    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-blue-500/40 bg-blue-500/10 px-2.5 py-1 text-xs font-medium text-blue-400">
                        <CheckCircle2 className="h-3.5 w-3.5" /> {t('selectedForLive')}
                    </span>
                ) : isConfigured ? (
                    <span className="rounded-full border border-[var(--af-border-strong)] px-2.5 py-1 text-xs font-medium text-[var(--af-text-2)]">
                        {t('clickToSelect')}
                    </span>
                ) : (
                    <span className="text-xs text-[var(--af-text-3)]">{t('externalSttNotConfigured')}</span>
                )}
            </div>

            <div
                className="space-y-4"
                onClick={(event) => event.stopPropagation()}
                onKeyDown={(event) => event.stopPropagation()}
            >
                <div className="space-y-1.5">
                    <Label htmlFor="external-stt-url">{t('externalSttUrlLabel')}</Label>
                    <Input
                        id="external-stt-url"
                        value={config.url}
                        placeholder="http://127.0.0.1:8080/v1/audio/transcriptions"
                        spellCheck={false}
                        onChange={(event) => update('url', event.target.value)}
                    />
                    <p className="text-xs text-[var(--af-text-3)]">{t('externalSttUrlHint')}</p>
                </div>

                <div className="grid gap-4 sm:grid-cols-2">
                    <div className="space-y-1.5">
                        <Label htmlFor="external-stt-mode">{t('externalSttModeLabel')}</Label>
                        <Select
                            value={config.mode}
                            onValueChange={(value) => update('mode', value as ExternalSttConfig['mode'])}
                        >
                            <SelectTrigger id="external-stt-mode">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value="multipart">{t('externalSttModeMultipart')}</SelectItem>
                                <SelectItem value="raw">{t('externalSttModeRaw')}</SelectItem>
                            </SelectContent>
                        </Select>
                    </div>

                    <div className="space-y-1.5">
                        <Label htmlFor="external-stt-response-path">{t('externalSttResponsePathLabel')}</Label>
                        <Input
                            id="external-stt-response-path"
                            value={config.responsePath}
                            placeholder="text"
                            spellCheck={false}
                            onChange={(event) => update('responsePath', event.target.value)}
                        />
                    </div>
                </div>
                <p className="text-xs text-[var(--af-text-3)]">{t('externalSttResponsePathHint')}</p>

                {config.mode === 'multipart' ? (
                    <div className="grid gap-4 sm:grid-cols-2">
                        <div className="space-y-1.5">
                            <Label htmlFor="external-stt-file-field">{t('externalSttFileFieldLabel')}</Label>
                            <Input
                                id="external-stt-file-field"
                                value={config.fileField}
                                placeholder="file"
                                spellCheck={false}
                                onChange={(event) => update('fileField', event.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="external-stt-model">{t('externalSttModelLabel')}</Label>
                            <Input
                                id="external-stt-model"
                                value={config.model ?? ''}
                                placeholder="gigaam-v3"
                                spellCheck={false}
                                onChange={(event) => update('model', event.target.value || null)}
                            />
                        </div>
                    </div>
                ) : null}

                <div className="grid gap-4 sm:grid-cols-2">
                    <div className="space-y-1.5">
                        <Label htmlFor="external-stt-timeout">{t('externalSttTimeoutLabel')}</Label>
                        <Input
                            id="external-stt-timeout"
                            type="number"
                            min={1}
                            max={600}
                            value={Math.round(config.timeoutMs / 1000)}
                            onChange={(event) => {
                                const seconds = Number(event.target.value);
                                update('timeoutMs', Number.isFinite(seconds) && seconds > 0 ? seconds * 1000 : 30000);
                            }}
                        />
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="external-stt-retries">{t('externalSttRetriesLabel')}</Label>
                        <Input
                            id="external-stt-retries"
                            type="number"
                            min={0}
                            max={10}
                            value={config.maxRetries}
                            onChange={(event) => {
                                const retries = Number(event.target.value);
                                update('maxRetries', Number.isFinite(retries) && retries >= 0 ? Math.floor(retries) : 2);
                            }}
                        />
                    </div>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                    <Button type="button" onClick={() => void save()} disabled={isSaving || isTesting}>
                        {isSaving ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                        {t('externalSttSave')}
                    </Button>
                    <Button
                        type="button"
                        variant="outline"
                        onClick={() => void testConnection()}
                        disabled={isSaving || isTesting || config.url.trim().length === 0}
                    >
                        {isTesting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                        {t('externalSttTest')}
                    </Button>
                    {saved ? (
                        <span className="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-500">
                            <CheckCircle2 className="h-3.5 w-3.5" /> {t('externalSttSaved')}
                        </span>
                    ) : null}
                </div>

                {testMessage ? (
                    <p className="inline-flex items-center gap-1.5 text-sm text-emerald-500">
                        <CheckCircle2 className="h-4 w-4 shrink-0" /> {testMessage}
                    </p>
                ) : null}
                {error ? (
                    <p className="inline-flex items-start gap-1.5 text-sm text-red-400">
                        <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" /> {error}
                    </p>
                ) : null}
            </div>
        </div>
    );
}
