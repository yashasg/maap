import { beforeEach, expect, test, vi } from 'vitest';
import { getBillingAccess, getTeamBillingScope } from '@/lib/load';
import prisma from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { GET, POST, PUT } from './route';

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
}));

vi.mock('@/lib/load', () => ({
  getBillingAccess: vi.fn(),
  getTeamBillingScope: vi.fn(),
}));

vi.mock('@/lib/prisma', () => {
  const tx = {
    $queryRawUnsafe: vi.fn(),
    $executeRawUnsafe: vi.fn(),
  };
  return {
    default: {
      client: {
        teamUser: { findFirst: vi.fn() },
        $queryRawUnsafe: vi.fn(),
      },
      transaction: vi.fn((callback: (client: typeof tx) => unknown) => callback(tx)),
      __tx: tx,
    },
  };
});

const parseRequestMock = vi.mocked(parseRequest);
const client = prisma.client as any;
const transactionMock = vi.mocked(prisma.transaction);
const tx = (prisma as any).__tx;

const user = { id: '11111111-1111-4111-8111-111111111111', role: 'user', isAdmin: false };
const teamId = '22222222-2222-4222-8222-222222222222';
const websiteId = '33333333-3333-4333-8333-333333333333';
const folderId = '44444444-4444-4444-8444-444444444444';

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(getBillingAccess).mockResolvedValue({ isPastDue: false, isOwner: false });
  vi.mocked(getTeamBillingScope).mockResolvedValue({ teamId, accountId: user.id });
});

test.each([undefined, teamId])(
  'past-due workspace denies listing, creation and moves (%s)',
  async scope => {
    vi.mocked(getBillingAccess).mockResolvedValue({ isPastDue: true, isOwner: true });
    client.teamUser.findFirst.mockResolvedValue({ role: 'team-owner' });
    parseRequestMock.mockResolvedValue({
      auth: { user },
      query: { teamId: scope },
      body: { teamId: scope, websiteId, folderId, name: 'Work' },
      error: undefined,
    });
    const request = new Request('http://localhost/api/workspace-folders');
    for (const handler of [GET, POST, PUT]) {
      expect((await handler(request)).status).toBe(401);
    }
    if (scope) expect(getTeamBillingScope).toHaveBeenCalledWith(scope);
    else expect(getBillingAccess).toHaveBeenCalledWith({ accountId: user.id }, user.id);
    expect(client.$queryRawUnsafe).not.toHaveBeenCalled();
    expect(transactionMock).not.toHaveBeenCalled();
  },
);

test('GET limits personal folders and assignments to live websites in the same scope', async () => {
  parseRequestMock.mockResolvedValue({ auth: { user }, query: {}, error: undefined });
  client.$queryRawUnsafe
    .mockResolvedValueOnce([{ id: folderId, name: 'Work' }])
    .mockResolvedValueOnce([{ websiteId, folderId }]);

  const response = await GET(new Request('http://localhost/api/workspace-folders'));

  expect(client.$queryRawUnsafe.mock.calls[0][0]).toContain(
    'WHERE user_id = $1 AND team_id IS NULL',
  );
  expect(client.$queryRawUnsafe.mock.calls[1][0]).toContain('w.deleted_at IS NULL');
  expect(client.$queryRawUnsafe.mock.calls[1][0]).toContain('w.user_id = $1 AND w.team_id IS NULL');
  expect(client.$queryRawUnsafe.mock.calls[0][1]).toBe(user.id);
  expect(client.$queryRawUnsafe.mock.calls[1][1]).toBe(user.id);
  expect(await response.json()).toEqual({
    folders: [{ id: folderId, name: 'Work' }],
    assignments: [{ websiteId, folderId }],
  });
});

test('GET allows a scoped team view-only member to read organization', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user },
    query: { teamId },
    error: undefined,
  });
  client.teamUser.findFirst.mockResolvedValue({ role: 'team-view-only' });
  client.$queryRawUnsafe.mockResolvedValue([]);

  const response = await GET(
    new Request(`http://localhost/api/workspace-folders?teamId=${teamId}`),
  );

  expect(client.teamUser.findFirst).toHaveBeenCalledWith({
    where: { teamId, userId: user.id, team: { deletedAt: null } },
    select: { role: true },
  });
  expect(response.status).toBe(200);
});

test('POST rejects personal and team view-only writes', async () => {
  parseRequestMock.mockResolvedValueOnce({
    auth: { user: { ...user, role: 'view-only' } },
    body: { name: 'Work' },
    error: undefined,
  });

  expect(
    (await POST(new Request('http://localhost/api/workspace-folders', { method: 'POST' }))).status,
  ).toBe(401);

  parseRequestMock.mockResolvedValueOnce({
    auth: { user },
    body: { teamId, name: 'Work' },
    error: undefined,
  });
  client.teamUser.findFirst.mockResolvedValue({ role: 'team-view-only' });

  expect(
    (await POST(new Request('http://localhost/api/workspace-folders', { method: 'POST' }))).status,
  ).toBe(401);
  expect(client.$queryRawUnsafe).not.toHaveBeenCalled();
});

test('POST creates a trimmed folder for a writable team scope', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user },
    body: { teamId, name: 'Roadmap' },
    error: undefined,
  });
  client.teamUser.findFirst.mockResolvedValue({ role: 'team-member' });
  client.$queryRawUnsafe.mockResolvedValue([{ id: folderId, name: 'Roadmap' }]);

  const response = await POST(
    new Request('http://localhost/api/workspace-folders', { method: 'POST' }),
  );

  expect(client.$queryRawUnsafe.mock.calls[0][0]).toContain(
    'VALUES ($1, $2, NULL, $3, now(), now())',
  );
  expect(client.$queryRawUnsafe.mock.calls[0].slice(2)).toEqual(['Roadmap', teamId]);
  expect(response.status).toBe(200);
});

test('PUT rejects deleted, transferred, and cross-scope resources', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user },
    body: { websiteId, folderId },
    error: undefined,
  });
  tx.$queryRawUnsafe.mockResolvedValueOnce([{ allowed: false }]);

  const missingWebsite = await PUT(
    new Request('http://localhost/api/workspace-folders', { method: 'PUT' }),
  );

  expect(tx.$queryRawUnsafe.mock.calls[0][0]).toContain('website_id = $1 AND user_id = $2');
  expect(tx.$queryRawUnsafe.mock.calls[0][0]).toContain('deleted_at IS NULL');
  expect(missingWebsite.status).toBe(401);
  expect(tx.$executeRawUnsafe).not.toHaveBeenCalled();

  tx.$queryRawUnsafe
    .mockResolvedValueOnce([{ allowed: true }])
    .mockResolvedValueOnce([{ allowed: false }]);

  const crossScopeFolder = await PUT(
    new Request('http://localhost/api/workspace-folders', { method: 'PUT' }),
  );

  expect(tx.$queryRawUnsafe.mock.calls[2][0]).toContain(
    'folder_id = $1 AND user_id = $2 AND team_id IS NULL',
  );
  expect(crossScopeFolder.status).toBe(401);
  expect(tx.$executeRawUnsafe).not.toHaveBeenCalled();
});

test('PUT moves a website to root by removing its assignment', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user },
    body: { websiteId, folderId: null },
    error: undefined,
  });
  tx.$queryRawUnsafe.mockResolvedValue([{ allowed: true }]);
  tx.$executeRawUnsafe.mockResolvedValue(1);

  const response = await PUT(
    new Request('http://localhost/api/workspace-folders', { method: 'PUT' }),
  );

  expect(transactionMock).toHaveBeenCalledTimes(1);
  expect(tx.$executeRawUnsafe).toHaveBeenCalledWith(
    'DELETE FROM website_folder_assignment WHERE website_id = $1',
    websiteId,
  );
  expect(response.status).toBe(200);
});

test('PUT assigns a website only after both resources match the workspace', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user },
    body: { websiteId, folderId },
    error: undefined,
  });
  tx.$queryRawUnsafe
    .mockResolvedValueOnce([{ allowed: true }])
    .mockResolvedValueOnce([{ allowed: true }]);
  tx.$executeRawUnsafe.mockResolvedValue(1);

  const response = await PUT(
    new Request('http://localhost/api/workspace-folders', { method: 'PUT' }),
  );

  expect(tx.$executeRawUnsafe.mock.calls[0][0]).toContain('ON CONFLICT (website_id) DO UPDATE');
  expect(tx.$executeRawUnsafe.mock.calls[0].slice(1)).toEqual([websiteId, folderId]);
  expect(response.status).toBe(200);
});
