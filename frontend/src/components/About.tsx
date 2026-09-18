import React, { useState, useEffect } from "react";
import { useTranslations } from 'next-intl';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import Image from 'next/image';
import { UpdateDialog } from "./UpdateDialog";
import { updateService, UpdateInfo } from '@/services/updateService';
import { UPDATES_AVAILABLE } from '@/lib/updates';
import { Button } from './ui/button';
import { Loader2, CheckCircle2 } from 'lucide-react';
import { toast } from 'sonner';
import { usePlatform } from '@/hooks/usePlatform';


export function About() {
    const t = useTranslations('about');
    const platform = usePlatform();
    const [currentVersion, setCurrentVersion] = useState<string>('0.0.1');
    const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
    const [isChecking, setIsChecking] = useState(false);
    const [showUpdateDialog, setShowUpdateDialog] = useState(false);

    useEffect(() => {
        // Get current version on mount
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    const handleCheckForUpdates = async () => {
        setIsChecking(true);
        try {
            const info = await updateService.checkForUpdates(true);
            setUpdateInfo(info);
            if (info.available) {
                setShowUpdateDialog(true);
            } else {
                toast.success(t('latestVersion'));
            }
        } catch (error: any) {
            console.error('Failed to check for updates:', error);
            toast.error(t('checkUpdatesFailed', { error: error.message || t('unknownError') }));
        } finally {
            setIsChecking(false);
        }
    };

    const openExternal = (url: string) => {
        invoke('open_external_url', { url }).catch((error) => {
            console.error('Failed to open external link:', error);
            toast.error(t('openLinkFailed'));
        });
    };

    return (
        <div className="p-4 space-y-4 h-[80vh] overflow-y-auto">
            {/* Compact Header */}
            <div className="text-center">
                <div className="mb-3">
                    <Image
                        src="icon_128x128.png"
                        alt="Talkkeeper Logo"
                        width={64}
                        height={64}
                        className="mx-auto"
                    />
                </div>
                {/* <h1 className="text-xl font-bold text-gray-900">Talkkeeper</h1> */}
                <span className="text-sm text-gray-500"> v{currentVersion}</span>
                <p className="text-medium text-gray-600 mt-1">
                    {t('tagline')}
                </p>
                <div className="mt-3">
                    {/* No update source of our own yet: point at the releases instead. */}
                    {platform === 'macos' || !UPDATES_AVAILABLE ? (
                        <Button
                            onClick={() => openExternal('https://github.com/leyvanah/Talkkeeper/releases')}
                            variant="outline"
                            size="sm"
                            className="text-xs"
                        >
                            <CheckCircle2 className="h-3 w-3 mr-2" />
                            {platform === 'macos' ? t('viewMacosReleases') : t('viewReleases')}
                        </Button>
                    ) : (
                        <Button
                            onClick={handleCheckForUpdates}
                            disabled={isChecking}
                            variant="outline"
                            size="sm"
                            className="text-xs"
                        >
                            {isChecking ? (
                                <>
                                    <Loader2 className="h-3 w-3 mr-2 animate-spin" />
                                    {t('checking')}
                                </>
                            ) : (
                                <>
                                    <CheckCircle2 className="h-3 w-3 mr-2" />
                                    {t('checkForUpdates')}
                                </>
                            )}
                        </Button>
                    )}
                    {updateInfo?.available && (
                        <div className="mt-2 text-xs text-blue-600">
                            {t('updateAvailable', { version: updateInfo.version ?? '' })}
                        </div>
                    )}
                </div>
            </div>

            {/* Features Grid - Compact */}
            <div className="space-y-3">
                <h2 className="text-base font-semibold text-gray-800">{t('whatMakesDifferent')}</h2>
                <div className="grid grid-cols-2 gap-2">
                    <div className="bg-gray-50 rounded p-3 hover:bg-gray-100 transition-colors">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('featurePrivacyTitle')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('featurePrivacyDetail')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3 hover:bg-gray-100 transition-colors">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('featureAnyModelTitle')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('featureAnyModelDetail')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3 hover:bg-gray-100 transition-colors">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('featureCostTitle')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('featureCostDetail')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3 hover:bg-gray-100 transition-colors">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('featureEverywhereTitle')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('featureEverywhereDetail')}</p>
                    </div>
                </div>
            </div>

            {/* Footer - Compact */}
            <div className="pt-2 border-t border-gray-200 text-center">
                <p className="text-xs text-gray-400">
                    {t('footerLicense')}
                </p>
                <p className="mt-1 text-xs text-gray-500">
                    {t.rich('developerCredit', {
                        handle: () => (
                            <button
                                type="button"
                                title={t('profileOnGithub')}
                                className="underline underline-offset-2 transition-colors hover:text-blue-500"
                                onClick={() => openExternal('https://github.com/leyvanah')}
                            >
                                leyvanah
                            </button>
                        ),
                    })}
                </p>
            </div>

            {/* Update Dialog */}
            <UpdateDialog
                open={showUpdateDialog}
                onOpenChange={setShowUpdateDialog}
                updateInfo={updateInfo}
            />
        </div>

    )
}
