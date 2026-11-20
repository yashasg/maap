import { renderHook } from '@testing-library/react';
import { expect, test, vi } from 'vitest';
import { useAppSettingsMutation, useAppSettingsQuery } from './useAppSettingsQuery';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  query: vi.fn((options: any) => options),
  mutation: vi.fn((options: any) => options),
  touch: vi.fn(),
}));

vi.mock('../useApi', () => ({
  useApi: () => ({
    get: mocks.get,
    post: mocks.post,
    useQuery: mocks.query,
    useMutation: mocks.mutation,
  }),
}));

vi.mock('../useModified', () => ({
  useModified: () => ({ modified: 2, touch: mocks.touch }),
}));

test('queries the selected app with an app-specific cache key', async () => {
  renderHook(() => useAppSettingsQuery('app-1'));
  const options = mocks.query.mock.lastCall[0];

  expect(options.queryKey).toEqual(['app-settings', { websiteId: 'app-1', modified: 2 }]);
  await options.queryFn();
  expect(mocks.get).toHaveBeenCalledWith('/websites/app-1/app-settings');
});

test('updates the complete app settings record through the same scoped endpoint', async () => {
  renderHook(() => useAppSettingsMutation('app-1'));
  const options = mocks.mutation.mock.lastCall[0];
  const settings = {
    overviewLayout: 'application',
    colorScheme: 'vibrant',
    revenueDisplayMode: 'revenuecat',
  };

  await options.mutationFn(settings);
  expect(mocks.post).toHaveBeenCalledWith('/websites/app-1/app-settings', settings);
});
