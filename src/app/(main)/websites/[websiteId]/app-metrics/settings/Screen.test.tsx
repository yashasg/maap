import type { ReactNode } from 'react';
import { expect, test, vi } from 'vitest';
import { render, screen, waitFor, within } from '@/test/render';
import Screen from './Screen';

const websiteMutateAsync = vi.fn();
const settingsMutateAsync = vi.fn();
const deleteMutateAsync = vi.fn();
const touch = vi.fn();
const toast = vi.fn();
const push = vi.fn();
const useUpdateQuery = vi.fn();
const useWebsite = vi.fn();
const useAppSettingsQuery = vi.fn();
const useAppSettingsMutation = vi.fn();

const translations = new Proxy({}, { get: (_, key) => String(key) });

vi.mock('@umami/react-zen', async importOriginal => {
  const actual = await importOriginal<typeof import('@umami/react-zen')>();
  return {
    ...actual,
    ListItem: ({ id, children }: { id: string; children: ReactNode }) => (
      <option value={id}>{children}</option>
    ),
    Select: ({
      value,
      onChange,
      children,
      isDisabled,
      'data-test': dataTest,
    }: {
      value?: string;
      onChange?: (value: string) => void;
      children: ReactNode;
      isDisabled?: boolean;
      'data-test'?: string;
    }) => (
      <select
        data-test={dataTest}
        value={value}
        disabled={isDisabled}
        onChange={event => onChange?.(event.target.value)}
      >
        {children}
      </select>
    ),
    useToast: () => ({ toast }),
  };
});

vi.mock('@/components/hooks', () => ({
  useAppSettingsMutation: () => useAppSettingsMutation(),
  useAppSettingsQuery: () => useAppSettingsQuery(),
  useDeleteQuery: () => ({
    mutateAsync: deleteMutateAsync,
    isPending: false,
    error: undefined,
    touch,
  }),
  useMessages: () => ({
    t: (value: string) => value,
    labels: translations,
    messages: translations,
    getErrorMessage: () => undefined,
  }),
  useNavigation: () => ({
    router: { push },
    renderUrl: (url: string) => url,
  }),
  useUpdateQuery: (url: string) => useUpdateQuery(url),
  useWebsite: () => useWebsite(),
}));

function mockSettings(canWrite: boolean) {
  useAppSettingsQuery.mockReturnValue({
    data: {
      websiteId: 'app-1',
      overviewLayout: 'application',
      colorScheme: 'default',
      revenueDisplayMode: 'none',
      canWrite,
    },
    dataUpdatedAt: 1,
    isLoading: false,
    error: undefined,
    refetch: vi.fn(),
  });
  useAppSettingsMutation.mockReturnValue({
    mutateAsync: settingsMutateAsync,
    isPending: false,
    error: undefined,
    touch,
  });
}

test('persists finite report preferences and keeps delete behind confirmation', async () => {
  useWebsite.mockReturnValue({
    id: 'app-1',
    name: 'Original App',
    domain: 'app.ios',
    userId: 'user-1',
    teamId: null,
  });
  mockSettings(true);
  useUpdateQuery.mockReturnValue({
    mutateAsync: websiteMutateAsync,
    error: undefined,
    isPending: false,
    touch,
  });
  websiteMutateAsync.mockImplementation(async (_data, options) => options?.onSuccess?.());
  settingsMutateAsync.mockImplementation(async (_data, options) => options?.onSuccess?.());

  const { user } = render(<Screen websiteId="app-1" />);

  await user.selectOptions(screen.getByTestId('settings-color-scheme'), 'high-contrast');
  await user.selectOptions(screen.getByTestId('settings-revenue-mode'), 'revenuecat');
  await user.click(screen.getByTestId('settings-save-preferences'));

  expect(settingsMutateAsync).toHaveBeenCalledWith(
    {
      overviewLayout: 'application',
      colorScheme: 'high-contrast',
      revenueDisplayMode: 'revenuecat',
    },
    expect.objectContaining({ onSuccess: expect.any(Function) }),
  );
  expect(touch).toHaveBeenCalledWith('app-settings:app-1');

  await user.click(screen.getByTestId('settings-delete'));
  const dialog = await screen.findByRole('dialog');
  expect(within(dialog).getByText('actionConfirmation')).toBeInTheDocument();
  await user.click(within(dialog).getByRole('button', { name: 'cancel' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  expect(deleteMutateAsync).not.toHaveBeenCalled();
});

test('uses the scoped website write for the app name', async () => {
  useWebsite.mockReturnValue({
    id: 'app-1',
    name: 'Original App',
    domain: 'app.ios',
    userId: 'user-1',
    teamId: null,
  });
  mockSettings(true);
  useUpdateQuery.mockReturnValue({
    mutateAsync: websiteMutateAsync,
    error: undefined,
    isPending: false,
    touch,
  });
  websiteMutateAsync.mockImplementation(async (_data, options) => options?.onSuccess?.());

  const { user } = render(<Screen websiteId="app-1" />);
  const name = screen.getByDisplayValue('Original App');
  await user.clear(name);
  await user.type(name, 'Renamed App');
  await user.click(screen.getByTestId('settings-save-name'));

  expect(useUpdateQuery).toHaveBeenCalledWith('/websites/app-1');
  expect(websiteMutateAsync).toHaveBeenCalledWith(
    { name: 'Renamed App' },
    expect.objectContaining({ onSuccess: expect.any(Function) }),
  );
});

test('prevents a read-only member from changing or deleting app settings', () => {
  useWebsite.mockReturnValue({
    id: 'app-1',
    name: 'Original App',
    domain: 'app.ios',
    userId: null,
    teamId: 'team-1',
  });
  mockSettings(false);
  useUpdateQuery.mockReturnValue({
    mutateAsync: websiteMutateAsync,
    error: undefined,
    isPending: false,
    touch,
  });

  render(<Screen websiteId="app-1" />);

  expect(screen.getByRole('status')).toHaveTextContent('Read-only access');
  expect(screen.getByTestId('settings-save-name')).toBeDisabled();
  expect(screen.getByTestId('settings-save-preferences')).toBeDisabled();
  expect(screen.getByTestId('settings-save-info')).toBeDisabled();
  expect(screen.getByTestId('settings-delete')).toBeDisabled();
  expect(screen.getByDisplayValue('Original App')).toHaveAttribute('readonly');
  expect(settingsMutateAsync).not.toHaveBeenCalled();
  expect(websiteMutateAsync).not.toHaveBeenCalled();
});
