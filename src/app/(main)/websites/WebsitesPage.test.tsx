import { beforeEach, expect, test, vi } from 'vitest';
import { render, screen, within } from '@/test/render';
import { WebsitesPage } from './WebsitesPage';

const moveWebsite = vi.fn();
const createFolder = vi.fn();
const push = vi.fn();
const replace = vi.fn();

const websites = [
  {
    id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
    name: 'Alpha',
    domain: 'app.ios',
    userId: 'user-1',
  },
  {
    id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
    name: 'Beta',
    domain: 'beta.example',
    userId: 'user-1',
  },
];

vi.mock('@/components/hooks', () => ({
  useDateParameters: () => ({ startAt: 1, endAt: 2 }),
  useLoginQuery: () => ({ user: { id: 'user-1', username: 'Acme', role: 'admin' } }),
  useLocale: () => ({ dir: 'ltr' }),
  useNavigation: () => ({
    teamId: undefined,
    query: { date: '30day', testMode: 'true' },
    router: { push, replace },
    updateParams: (params: Record<string, unknown>) =>
      `/websites?${new URLSearchParams(
        Object.entries(params)
          .filter(([, value]) => value !== undefined)
          .map(([key, value]) => [key, String(value)]),
      )}`,
    renderUrl: (path: string, params?: false) =>
      params === false ? path : `${path}?date=30day&testMode=true`,
  }),
  useTeamMembersQuery: () => ({ data: undefined }),
  useTeamQuery: () => ({ data: undefined }),
  useTimezone: () => ({ timezone: 'UTC', saveTimezone: vi.fn() }),
  useMobile: () => ({ isMobile: false }),
  useUserWebsitesQuery: () => ({
    data: { data: websites, count: 2 },
    isLoading: false,
  }),
}));

vi.mock('@/components/input/WebsiteDateFilter', () => ({
  WebsiteDateFilter: () => <button type="button">Last 30 days</button>,
}));

vi.mock('./WebsiteAddButton', () => ({
  WebsiteAddButton: () => <button type="button">Create new app or website</button>,
}));

vi.mock('./WorkspacePoll', () => ({
  WorkspacePoll: () => <aside>Workspace survey</aside>,
}));

vi.mock('./useWorkspaceHome', () => ({
  useWorkspaceCardMetrics: () => ({
    data: {
      data: [
        { websiteId: websites[0].id, users: 12, errors: 1 },
        { websiteId: websites[1].id, users: 4, errors: 0 },
      ],
    },
    isFetching: false,
  }),
  useWorkspaceFolders: () => ({
    folders: [{ id: 'folder-1', name: 'Production' }],
    assignments: [{ websiteId: websites[1].id, folderId: 'folder-1' }],
    createFolder: { mutateAsync: createFolder, isPending: false, error: null },
    moveWebsite: { mutateAsync: moveWebsite, isPending: false },
  }),
  useWorkspaceHeartbeat: () => ({ data: { ok: true }, isError: false }),
}));

beforeEach(() => {
  moveWebsite.mockReset();
  moveWebsite.mockResolvedValue({});
  createFolder.mockReset();
  createFolder.mockResolvedValue({ id: 'folder-2', name: 'Experiments' });
  push.mockReset();
  replace.mockReset();
});

function getWebsiteCard(name: string) {
  const card = screen.getAllByTestId('website-card').find(item => item.textContent?.includes(name));

  if (!card) {
    throw new Error(`Could not find website card: ${name}`);
  }

  return card;
}

test('renders grouped app cards with live metrics and preserved report controls', () => {
  render(<WebsitesPage />);

  expect(screen.getByRole('heading', { name: 'Acme' })).toBeVisible();
  expect(screen.getByText('2 apps')).toBeVisible();
  expect(screen.getByText('TEST DATA')).toBeVisible();
  expect(screen.getByText('Workspace survey')).toBeVisible();

  const alpha = getWebsiteCard('Alpha');
  expect(within(alpha).getByText('12')).toBeVisible();
  expect(within(alpha).getByText('1')).toBeVisible();
  expect(within(alpha).getByRole('link', { name: 'Alpha' })).toHaveAttribute(
    'href',
    `/websites/${websites[0].id}?date=30day&testMode=true`,
  );

  expect(screen.getByText('Production', { selector: 'summary' })).toBeVisible();
});

test('moves an app into a folder', async () => {
  const { user } = render(<WebsitesPage />);
  const alpha = getWebsiteCard('Alpha');

  await user.selectOptions(within(alpha).getByRole('combobox', { name: 'Move Alpha to folder' }), [
    'folder-1',
  ]);

  expect(moveWebsite).toHaveBeenCalledWith({
    websiteId: websites[0].id,
    folderId: 'folder-1',
  });
});

test('creates a workspace folder', async () => {
  const { user } = render(<WebsitesPage />);

  await user.click(screen.getByRole('button', { name: 'New folder' }));
  const dialog = screen.getByRole('dialog');
  await user.type(within(dialog).getByLabelText('Folder name'), 'Experiments');
  await user.click(within(dialog).getByRole('button', { name: 'Create folder' }));

  expect(createFolder).toHaveBeenCalledWith('Experiments');
});
