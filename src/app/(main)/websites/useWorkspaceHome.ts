'use client';

import { useQueryClient } from '@tanstack/react-query';
import { useApi } from '@/components/hooks/useApi';

const CARD_METRICS_BATCH_SIZE = 20;

export type WorkspaceFolder = {
  id: string;
  name: string;
};

export type WorkspaceFolderAssignment = {
  websiteId: string;
  folderId: string;
};

export type WorkspaceCardMetric = {
  websiteId: string;
  users: number | null;
  errors: number | null;
};

export function useWorkspaceFolders(teamId?: string) {
  const { get, post, put, useMutation, useQuery } = useApi();
  const queryClient = useQueryClient();
  const queryKey = ['workspace-folders', { teamId }];
  const query = useQuery<{
    folders: WorkspaceFolder[];
    assignments: WorkspaceFolderAssignment[];
  }>({
    queryKey,
    queryFn: () => get('/workspace-folders', teamId ? { teamId } : {}),
  });

  const createFolder = useMutation({
    mutationFn: (name: string) =>
      post('/workspace-folders', {
        ...(teamId ? { teamId } : {}),
        name,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  const moveWebsite = useMutation({
    mutationFn: ({ websiteId, folderId }: { websiteId: string; folderId: string | null }) =>
      put('/workspace-folders', {
        ...(teamId ? { teamId } : {}),
        websiteId,
        folderId,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  return {
    ...query,
    folders: query.data?.folders ?? [],
    assignments: query.data?.assignments ?? [],
    createFolder,
    moveWebsite,
  };
}

export function useWorkspaceCardMetrics({
  websiteIds,
  startAt,
  endAt,
  testMode,
  teamId,
}: {
  websiteIds: string[];
  startAt: number;
  endAt: number;
  testMode: boolean;
  teamId?: string;
}) {
  const { post, useQuery } = useApi();

  return useQuery<{ data: WorkspaceCardMetric[] }>({
    queryKey: [
      'workspace-card-metrics',
      { websiteIds: [...websiteIds].sort(), startAt, endAt, testMode, teamId },
    ],
    queryFn: async () => {
      const batches = Array.from(
        { length: Math.ceil(websiteIds.length / CARD_METRICS_BATCH_SIZE) },
        (_, index) =>
          websiteIds.slice(index * CARD_METRICS_BATCH_SIZE, (index + 1) * CARD_METRICS_BATCH_SIZE),
      );
      const responses = await Promise.all(
        batches.map(batch =>
          post('/websites/card-metrics', {
            websiteIds: batch,
            startAt,
            endAt,
            testMode,
            ...(teamId ? { teamId } : {}),
          }),
        ),
      );

      return {
        data: responses.flatMap(response => response.data ?? []),
      };
    },
    enabled:
      websiteIds.length > 0 &&
      Number.isFinite(startAt) &&
      Number.isFinite(endAt) &&
      endAt >= startAt,
  });
}

export function useWorkspaceHeartbeat() {
  const { get, useQuery } = useApi();

  return useQuery<{ ok: boolean }>({
    queryKey: ['heartbeat'],
    queryFn: () => get('/heartbeat'),
    retry: 1,
    refetchInterval: 60_000,
  });
}
