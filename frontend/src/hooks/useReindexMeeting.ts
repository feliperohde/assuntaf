'use client';

import { useCallback, useState } from 'react';
import { toast } from 'sonner';
import { useI18n } from '@/i18n';
import { ragService } from '@/services/ragService';

/**
 * Reindexes one meeting into the knowledge index right away. Passages are
 * searchable when the call returns; tickets/decisions follow in the background.
 */
export function useReindexMeeting() {
  const { t } = useI18n();
  const [reindexingId, setReindexingId] = useState<string | null>(null);

  const reindex = useCallback(
    async (meetingId: string) => {
      setReindexingId(meetingId);
      try {
        const outcome = await ragService.indexMeeting(meetingId);
        if (!outcome) {
          toast.info(t('reindex.disabled'));
        } else if (outcome.status === 'indexed') {
          toast.success(t('reindex.done', { count: outcome.chunkCount }), { description: t('reindex.factsLater') });
        } else if (outcome.status === 'partial') {
          toast.warning(t('reindex.partial', { count: outcome.chunkCount }), { description: outcome.error ?? undefined });
        } else {
          toast.error(t('reindex.failed'), { description: outcome.error ?? undefined });
        }
      } catch (error) {
        toast.error(t('reindex.failed'), { description: String(error) });
      } finally {
        setReindexingId(null);
      }
    },
    [t]
  );

  return { reindex, reindexingId };
}
