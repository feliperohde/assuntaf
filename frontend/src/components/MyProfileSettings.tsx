'use client';

import React, { useEffect, useState } from 'react';
import { UserRound } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useI18n } from '@/i18n';
import { UserProfile, meService } from '@/services/meService';

/** The user's name (labels their lines) and what the app learned of their voice. */
export function MyProfileSettings() {
  const { t } = useI18n();
  const [profile, setProfile] = useState<UserProfile | null>(null);
  const [name, setName] = useState('');

  useEffect(() => {
    meService
      .getProfile()
      .then(p => {
        setProfile(p);
        setName(p.displayName ?? '');
      })
      .catch(error => console.error('Failed to load profile:', error));
  }, []);

  const save = async () => {
    try {
      setProfile(await meService.setDisplayName(name.trim() || null));
      toast.success(t('myTime.nameSaved'));
    } catch (error) {
      toast.error(String(error));
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <UserRound className="h-4 w-4 text-gray-600" />
        <h4 className="text-sm font-medium text-gray-900">{t('profile.title')}</h4>
      </div>
      <div className="flex gap-2 max-w-md">
        <Input value={name} onChange={e => setName(e.target.value)} placeholder={t('myTime.namePlaceholder')} />
        <Button variant="outline" onClick={save} disabled={name.trim() === (profile?.displayName ?? '')}>
          {t('common.save')}
        </Button>
      </div>
      <p className="text-xs text-gray-500">
        {t('profile.help')}{' '}
        {profile && profile.voiceprintSamples > 0
          ? t('profile.voiceLearned', { count: profile.voiceprintSamples })
          : t('profile.voiceNotYet')}
      </p>
    </div>
  );
}
