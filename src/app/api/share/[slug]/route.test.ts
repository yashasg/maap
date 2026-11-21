import { beforeEach, expect, test, vi } from 'vitest';
import { resolveBoardShareScope } from '@/lib/board-share';
import { ENTITY_TYPE } from '@/lib/constants';
import { createToken } from '@/lib/jwt';
import { getShareByCode } from '@/queries/prisma';
import { GET } from './route';

vi.mock('@/lib/board-share', () => ({ resolveBoardShareScope: vi.fn() }));
vi.mock('@/lib/crypto', () => ({ secret: () => 'app-secret' }));
vi.mock('@/lib/jwt', () => ({ createToken: vi.fn(() => 'share-token') }));
vi.mock('@/lib/redis', () => ({ default: { enabled: false } }));
vi.mock('@/lib/prisma', () => ({
  default: { client: { teamUser: { findFirst: vi.fn().mockResolvedValue(null) } } },
}));
vi.mock('@/queries/prisma', () => ({
  getShareByCode: vi.fn(),
}));

async function mintBoardShare() {
  vi.mocked(getShareByCode).mockResolvedValue({
    id: 'share-1',
    shareType: ENTITY_TYPE.board,
    entityId: 'board-1',
    parameters: {},
  } as any);

  const response = await GET(new Request('http://localhost/api/share/slug'), {
    params: Promise.resolve({ slug: 'slug' }),
  });

  return {
    response,
    token: vi.mocked(createToken).mock.calls[0]?.[0] as Record<string, any> | undefined,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(resolveBoardShareScope).mockResolvedValue({
    board: { id: 'board-1', userId: 'user-1', teamId: null } as any,
    scope: {
      websiteIds: ['current-site'],
      pixelIds: ['current-pixel'],
      linkIds: ['current-link'],
    },
  });
});

test('a board share token is issued with the current resolved entity scope', async () => {
  const { response, token } = await mintBoardShare();

  expect(response.status).toBe(200);
  expect(token?.websiteIds).toEqual(['current-site']);
  expect(token?.pixelIds).toEqual(['current-pixel']);
  expect(token?.linkIds).toEqual(['current-link']);
});

test('a board share fails closed when its live owner or scope is invalid', async () => {
  vi.mocked(resolveBoardShareScope).mockResolvedValue(null);

  const { response, token } = await mintBoardShare();

  expect(response.status).toBe(404);
  expect(token).toBeUndefined();
});
