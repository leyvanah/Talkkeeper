'use client';

import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { useTranslations } from 'next-intl';
import { useWindowBack, useWindowTitle } from '@/components/AppHeader';
import { Settings2, Mic, Database as DatabaseIcon, SparkleIcon, Radar, Info, Cpu, Lock } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { motion } from 'framer-motion';
import { TranscriptSettings } from '@/components/TranscriptSettings';
import { RecordingSettings } from '@/components/RecordingSettings';
import { PreferenceSettings } from '@/components/PreferenceSettings';
import { SummaryModelSettings } from '@/components/SummaryModelSettings';
import { MeetingDetectionSettings } from '@/components/MeetingDetectionSettings';
import { DiarizationSettings } from '@/components/DiarizationSettings';
import { AboutSettings } from '@/components/AboutSettings';
import { BetaSettings } from '@/components/BetaSettings';
import { LocalStackStatus } from '@/components/LocalStackStatus';
import { SecuritySettings } from '@/components/SecuritySettings';
import { LibraryExportCard } from '@/components/security/LibraryExportCard'
import { useConfig } from '@/contexts/ConfigContext';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { pickSettingsTab, rememberSettingsTab, rememberedSettingsTab } from '@/lib/settings-tab';

// Labels are message keys; they are resolved per render so the tab rail
// follows the active locale.
const TABS = [
  { value: 'general', labelKey: 'tabGeneral', icon: Settings2 },
  { value: 'recording', labelKey: 'tabRecording', icon: Mic },
  { value: 'Transcriptionmodels', labelKey: 'tabTranscription', icon: DatabaseIcon },
  { value: 'summaryModels', labelKey: 'tabSummary', icon: SparkleIcon },
  { value: 'meetingDetection', labelKey: 'tabDetection', icon: Radar },
  { value: 'security', labelKey: 'tabSecurity', icon: Lock },
  { value: 'localStack', labelKey: 'tabLocalStack', icon: Cpu },
  { value: 'about', labelKey: 'tabAbout', icon: Info },
] as const;

const TAB_VALUES: readonly string[] = TABS.map((tab) => tab.value);

export default function SettingsPage() {
  const t = useTranslations('settings');
  useWindowTitle(t('pageTitle'));
  const router = useRouter();
  useWindowBack(t('back'), () => router.back());
  const { transcriptModelConfig, setTranscriptModelConfig } = useConfig();

  const [activeTab, setActiveTab] = useState(() =>
    pickSettingsTab(null, rememberedSettingsTab(), TAB_VALUES, 'general')
  );
  const contentScrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const requestedTab = sessionStorage.getItem('meetily-settings-tab');
    if (requestedTab && TAB_VALUES.includes(requestedTab)) {
      setActiveTab(requestedTab);
    }
    sessionStorage.removeItem('meetily-settings-tab');
  }, []);

  useEffect(() => {
    rememberSettingsTab(activeTab);
  }, [activeTab]);

  useEffect(() => {
    const loadTranscriptConfig = async () => {
      try {
        const config = await invoke('api_get_transcript_config') as any;
        if (config) {
          setTranscriptModelConfig({
            provider: config.provider || 'localWhisper',
            model: config.model || 'large-v3',
            apiKey: config.apiKey || null
          });
        }
      } catch (error) {
        console.error('Failed to load transcript config:', error);
      }
    };
    loadTranscriptConfig();
  }, [setTranscriptModelConfig]);

  // Each tab opens at its top instead of inheriting the previous tab's scroll position.
  useLayoutEffect(() => {
    contentScrollRef.current?.scrollTo({ top: 0 });
  }, [activeTab]);

  return (
    <div className="flex h-full min-h-0 min-w-0 max-w-full overflow-hidden bg-gray-50">
      <Tabs
        value={activeTab}
        onValueChange={setActiveTab}
        orientation="vertical"
        className="flex min-h-0 min-w-0 flex-1"
      >
        {/* Vertical tab rail: icons only on narrow windows, icons + labels from md up */}
        <nav className="no-scrollbar w-14 shrink-0 overflow-y-auto overflow-x-hidden border-r border-[var(--af-border)] px-2 py-4 md:w-56 md:px-3 md:py-6">
          <TabsList className="flex h-auto w-full flex-col items-stretch justify-start gap-1 rounded-none bg-transparent p-0">
            {TABS.map((tab) => {
              const Icon = tab.icon;
              const label = t(tab.labelKey);
              return (
                <TabsTrigger
                  key={tab.value}
                  value={tab.value}
                  title={label}
                  aria-label={label}
                  className="group relative flex h-10 w-full items-center justify-center gap-3 rounded-[var(--af-radius-sm)] border-0 bg-transparent px-0 text-sm font-medium text-[var(--af-text-2)] shadow-none transition-colors hover:bg-[var(--af-hover)] hover:text-[var(--af-text)] focus-visible:ring-offset-0 data-[state=active]:bg-transparent data-[state=active]:text-[var(--af-text)] data-[state=active]:shadow-none md:justify-start md:px-3"
                >
                  {tab.value === activeTab && (
                    <motion.span
                      aria-hidden
                      layoutId="settings-tab-active"
                      className="absolute inset-0 rounded-[var(--af-radius-sm)] bg-[var(--af-active)]"
                      transition={{ type: 'spring', stiffness: 500, damping: 40 }}
                    />
                  )}
                  <Icon className="relative z-10 h-4 w-4 shrink-0 group-data-[state=active]:text-[var(--af-accent)]" />
                  <span className="relative z-10 hidden truncate md:inline">{label}</span>
                </TabsTrigger>
              );
            })}
          </TabsList>
        </nav>

        {/* Body */}
        <div ref={contentScrollRef} className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
          <div className="mx-auto w-full max-w-5xl min-w-0 break-words px-4 py-4 sm:px-6 sm:py-6 lg:px-8">
            <TabsContent value="general" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <PreferenceSettings />
            </TabsContent>
            <TabsContent value="recording" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <RecordingSettings />
              <div className="mt-6">
                <BetaSettings />
              </div>
            </TabsContent>
            <TabsContent value="Transcriptionmodels" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <TranscriptSettings
                transcriptModelConfig={transcriptModelConfig}
                setTranscriptModelConfig={setTranscriptModelConfig}
              />
              <div className="mt-6">
                <DiarizationSettings />
              </div>
            </TabsContent>
            <TabsContent value="summaryModels" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <SummaryModelSettings />
            </TabsContent>
            <TabsContent value="meetingDetection" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <MeetingDetectionSettings />
            </TabsContent>
            <TabsContent value="security" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <SecuritySettings />
              <div className="mx-auto mt-6 max-w-2xl">
                <LibraryExportCard />
              </div>
            </TabsContent>
            <TabsContent value="localStack" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <LocalStackStatus />
            </TabsContent>
            <TabsContent value="about" className="mt-0 min-w-0 max-w-full focus-visible:ring-0">
              <AboutSettings />
            </TabsContent>
          </div>
        </div>
      </Tabs>
    </div>
  );
}
