import { beforeEach, describe, expect, test, vi } from 'vitest';
import { getBoardEntityIds } from '@/lib/boards';
import { getBillingAccess, getEntityBillingScope } from '@/lib/load';
import prisma from '@/lib/prisma';
import { getBoard, getUser } from '@/queries/prisma';
import { resolveBoardShareScope } from './board-share';

vi.mock('@/lib/boards', () => ({
  getBoardEntityIds: vi.fn(),
}));

vi.mock('@/lib/load', () => ({
  getBillingAccess: vi.fn(),
  getEntityBillingScope: vi.fn(),
}));

const prismaMocks = vi.hoisted(() => ({
  findTeam: vi.fn(),
  findWebsites: vi.fn(),
  findPixels: vi.fn(),
  findLinks: vi.fn(),
}));

vi.mock('@/lib/prisma', () => ({
  default: {
    client: {
      team: { findFirst: prismaMocks.findTeam },
      website: { findMany: prismaMocks.findWebsites },
      pixel: { findMany: prismaMocks.findPixels },
      link: { findMany: prismaMocks.findLinks },
    },
  },
}));

vi.mock('@/queries/prisma', () => ({
  getBoard: vi.fn(),
  getUser: vi.fn(),
}));

const getBoardMock = vi.mocked(getBoard);
const getBoardEntityIdsMock = vi.mocked(getBoardEntityIds);
const getBillingAccessMock = vi.mocked(getBillingAccess);
const getEntityBillingScopeMock = vi.mocked(getEntityBillingScope);
const getUserMock = vi.mocked(getUser);
const prismaMock = prisma as unknown as {
  client: {
    team: { findFirst: ReturnType<typeof vi.fn> };
    website: { findMany: ReturnType<typeof vi.fn> };
    pixel: { findMany: ReturnType<typeof vi.fn> };
    link: { findMany: ReturnType<typeof vi.fn> };
  };
};

function mockBoard(overrides: Record<string, unknown> = {}) {
  getBoardMock.mockResolvedValue({
    id: 'board-1',
    type: 'dashboard',
    name: 'Board',
    description: '',
    parameters: {},
    userId: 'user-1',
    teamId: null,
    createdAt: new Date(),
    updatedAt: new Date(),
    ...overrides,
  } as any);
}

beforeEach(() => {
  vi.clearAllMocks();
  mockBoard();
  getUserMock.mockResolvedValue({ id: 'user-1', username: 'owner', role: 'user' } as any);
  getEntityBillingScopeMock.mockImplementation(async entity => ({
    accountId: (entity as any).id,
    teamId: entity.teamId,
  }));
  getBillingAccessMock.mockResolvedValue({ isPastDue: false, isOwner: false });
  getBoardEntityIdsMock.mockReturnValue({
    websiteIds: ['retained-site', 'removed-site', 'deleted-site'],
    pixelIds: ['retained-pixel'],
    linkIds: ['retained-link'],
  });
  prismaMocks.findWebsites.mockResolvedValue([
    { id: 'retained-site', userId: 'user-1', teamId: null },
  ]);
  prismaMocks.findPixels.mockResolvedValue([
    { id: 'retained-pixel', userId: 'user-1', teamId: null },
  ]);
  prismaMocks.findLinks.mockResolvedValue([
    { id: 'retained-link', userId: 'user-1', teamId: null },
  ]);
});

describe('resolveBoardShareScope', () => {
  test('uses the personal owner current access and excludes removed or deleted entities', async () => {
    const result = await resolveBoardShareScope('board-1');

    expect(result?.scope).toEqual({
      websiteIds: ['retained-site'],
      pixelIds: ['retained-pixel'],
      linkIds: ['retained-link'],
    });
    expect(prismaMock.client.website.findMany).toHaveBeenCalledWith({
      where: {
        id: { in: ['retained-site', 'removed-site', 'deleted-site'] },
        deletedAt: null,
        OR: [
          { userId: 'user-1' },
          {
            team: {
              deletedAt: null,
              members: { some: { userId: 'user-1' } },
            },
          },
        ],
      },
      select: { id: true, userId: true, teamId: true },
    });
  });

  test('uses the board current team after a transfer and only includes team-owned entities', async () => {
    mockBoard({ userId: null, teamId: 'team-2' });
    prismaMocks.findTeam.mockResolvedValue({ id: 'team-2' });
    prismaMocks.findWebsites.mockResolvedValue([
      { id: 'removed-site', userId: null, teamId: 'team-2' },
    ]);

    const result = await resolveBoardShareScope('board-1');

    expect(result?.scope.websiteIds).toEqual(['removed-site']);
    expect(prismaMock.client.website.findMany).toHaveBeenCalledWith({
      where: {
        id: { in: ['retained-site', 'removed-site', 'deleted-site'] },
        deletedAt: null,
        teamId: 'team-2',
      },
      select: { id: true, userId: true, teamId: true },
    });
    expect(getBillingAccessMock).not.toHaveBeenCalled();
  });

  test('keeps personal-board billing restrictions from canonical share issuance', async () => {
    getBillingAccessMock.mockImplementation(async scope => ({
      isPastDue: scope.accountId === 'retained-site',
      isOwner: false,
    }));

    const result = await resolveBoardShareScope('board-1');

    expect(result?.scope.websiteIds).toEqual([]);
    expect(result?.scope.pixelIds).toEqual(['retained-pixel']);
  });

  test.each([
    ['a missing board', null],
    ['a board without an owner', { userId: null, teamId: null }],
    ['a board with conflicting owners', { userId: 'user-1', teamId: 'team-1' }],
  ])('fails closed for %s', async (_name, board) => {
    if (board === null) {
      getBoardMock.mockResolvedValue(null);
    } else {
      mockBoard(board);
    }

    await expect(resolveBoardShareScope('board-1')).resolves.toBeNull();
    expect(prismaMock.client.website.findMany).not.toHaveBeenCalled();
  });

  test('fails closed when a personal board owner has been deleted', async () => {
    getUserMock.mockResolvedValue(null);

    await expect(resolveBoardShareScope('board-1')).resolves.toBeNull();
    expect(prismaMock.client.website.findMany).not.toHaveBeenCalled();
  });

  test('fails closed when a team board owner has been deleted', async () => {
    mockBoard({ userId: null, teamId: 'team-1' });
    prismaMocks.findTeam.mockResolvedValue(null);

    await expect(resolveBoardShareScope('board-1')).resolves.toBeNull();
    expect(prismaMock.client.website.findMany).not.toHaveBeenCalled();
  });
});
