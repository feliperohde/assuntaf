import React, { useState, useEffect } from "react";
import { getVersion } from '@tauri-apps/api/app';
import { AssuntaWordmark } from './AssuntaMark';
import AnalyticsConsentSwitch from "./AnalyticsConsentSwitch";
import { UpdateDialog } from "./UpdateDialog";
import { updateService, UpdateInfo } from '@/services/updateService';
import { Button } from './ui/button';
import { Loader2, CheckCircle2 } from 'lucide-react';
import { toast } from 'sonner';
import { useI18n } from '@/i18n';


export function About() {
  const { t } = useI18n();
    const [currentVersion, setCurrentVersion] = useState<string>('0.4.1');
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
                toast.success(t('about.latest'));
            }
        } catch (error: any) {
            console.error('Failed to check for updates:', error);
            toast.error('Failed to check for updates: ' + (error.message || 'Unknown error'));
        } finally {
            setIsChecking(false);
        }
    };

    return (
        <div className="p-4 space-y-4 h-[80vh] overflow-y-auto">
            {/* Compact Header */}
            <div className="text-center">
                <div className="mb-2 flex justify-center">
                    <AssuntaWordmark size={44} />
                </div>
                <span className="text-sm text-gray-500"> v{currentVersion}</span>
                <p className="text-medium text-gray-600 mt-1">
                    {t('about.tagline')}
                </p>
                <div className="mt-3">
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
                                {t('about.checking')}
                            </>
                        ) : (
                            <>
                                <CheckCircle2 className="h-3 w-3 mr-2" />
                                {t('about.checkUpdates')}
                            </>
                        )}
                    </Button>
                    {updateInfo?.available && (
                        <div className="mt-2 text-xs text-blue-600">
                            Update available: v{updateInfo.version}
                        </div>
                    )}
                </div>
            </div>

            {/* Features */}
            <div className="space-y-3">
                <h2 className="text-base font-semibold text-gray-800">{t('about.whatItDoes')}</h2>
                <div className="grid grid-cols-2 gap-2">
                    <div className="bg-gray-50 rounded p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('nav.projects')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('about.projects')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('about.ask')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('about.askHelp')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('about.tickets')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('about.ticketsHelp')}</p>
                    </div>
                    <div className="bg-gray-50 rounded p-3">
                        <h3 className="font-bold text-sm text-gray-900 mb-1">{t('about.who')}</h3>
                        <p className="text-xs text-gray-600 leading-relaxed">{t('about.whoHelp')}</p>
                    </div>
                </div>
                <p className="text-xs text-gray-500">
                    Privacy-first: transcription, search and speaker detection run locally; summaries and answers use the
                    model you choose (local Ollama or an API).
                </p>
            </div>

            {/* Footer */}
            <div className="pt-2 border-t border-gray-200 text-center">
                <p className="text-xs text-gray-400">
                    {t('about.basedOn')}
                </p>
            </div>
            <AnalyticsConsentSwitch />

            {/* Update Dialog */}
            <UpdateDialog
                open={showUpdateDialog}
                onOpenChange={setShowUpdateDialog}
                updateInfo={updateInfo}
            />
        </div>

    )
}