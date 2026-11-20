import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { fetchAccount, getBillingAccess } from '@/lib/load';
import prisma from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { canViewTeam } from '@/permissions';
import { POST } from './route';

vi.mock('@/lib/prisma', () => ({
  default: {
    client: {
      website: {
        findMany: vi.fn(),
      },
    },
    rawQuery: vi.fn(),
  },
}));

vi.mock('@/lib/load', () => ({
  fetchAccount: vi.fn(),
  getBillingAccess: vi.fn(),
}));

vi.mock('@/lib/request', async importOriginal => ({
  ...(await importOriginal<typeof import('@/lib/request')>()),
  parseRequest: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canViewTeam: vi.fn(),
}));

const ids = ['11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222'];
const findMany = vi.mocked(prisma.client.website.findMany) as any;
const rawQuery = vi.mocked(prisma.rawQuery);
const parse = vi.mocked(parseRequest);
const billingAccess = vi.mocked(getBillingAccess);
const fetchAccountMock = vi.mocked(fetchAccount);

function parsed(body: Record<string, unknown>, auth: Record<string, unknown> = {}) {
  parse.mockResolvedValue({
    auth: {
      user: { id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', isAdmin: false },
      ...auth,
    },
    body,
    error: undefined,
  } as any);
}

beforeEach(() => {
  vi.unstubAllEnvs();
  vi.resetAllMocks();
  billingAccess.mockResolvedValue({ isPastDue: false, isOwner: true });
  fetchAccountMock.mockResolvedValue({ hasSubscription: true } as any);
  parsed({
    websiteIds: ids,
    startAt: Date.UTC(2026, 10, 1),
    endAt: Date.UTC(2026, 10, 20),
    testMode: false,
  });

  afterEach(() => {
    vi.useRealTimers();
  });
  findMany.mockResolvedValue([
    { id: ids[0], domain: 'app.ios' },
    { id: ids[1], domain: 'example.com' },
  ] as any);
  rawQuery.mockResolvedValue([
    { websiteId: ids[1], users: 9n, errors: 4n },
    { websiteId: ids[0], users: 3n, errors: 2n },
  ]);
});

test('validates unique IDs and a bounded ordered date range', async () => {
  parse.mockImplementation(async (_request, schema) => {
    const result = schema.safeParse({
      websiteIds: [ids[0], ids[0]],
      startAt: Date.UTC(2025, 0, 1),
      endAt: Date.UTC(2026, 1, 1),
      testMode: false,
    });
    return {
      auth: { user: { id: 'user-1' } },
      body: result.success ? result.data : undefined,
      error: result.success
        ? undefined
        : () => Response.json({ error: { code: 'bad-request' } }, { status: 400 }),
    } as any;
  });

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );

  expect(response.status).toBe(400);
  expect(findMany).not.toHaveBeenCalled();
  expect(rawQuery).not.toHaveBeenCalled();
});

test('returns one ordered result per requested website from one batch metrics query', async () => {
  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );
  const body = await response.json();

  expect(response.status).toBe(200);
  expect(body).toEqual({
    data: [
      { websiteId: ids[0], users: 3, errors: 2 },
      { websiteId: ids[1], users: 9, errors: 4 },
    ],
  });
  expect(rawQuery).toHaveBeenCalledTimes(1);

  const [sql, params] = rawQuery.mock.calls[0];
  expect(sql).toContain("e.event_name = 'TelemetryDeck.Error.occurred'");
  expect(sql).toContain('having count(distinct user_id) = 1');
  expect(sql).toContain('from error_event ee');
  expect(sql).toContain("greatest({{startDate}}, coalesce(w.reset_at, 'epoch'::timestamptz))");
  expect(sql).toContain("ee.created_at > coalesce(w.reset_at, 'epoch'::timestamptz)");
  expect(sql).not.toContain('TelemetryDeck.Error.id');
  expect(params).toMatchObject({
    websiteIds: ids,
    nativeWebsiteIds: [ids[0]],
    testMode: false,
  });
});

test('personal workspace accepts only current-user personal websites', async () => {
  await POST(new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }));

  expect(findMany).toHaveBeenCalledWith(
    expect.objectContaining({
      where: expect.objectContaining({
        userId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
        teamId: null,
      }),
    }),
  );
  expect(billingAccess).toHaveBeenCalledWith(
    { accountId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' },
    'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  );
});

test('rejects personal metrics when canonical billing access is past due', async () => {
  billingAccess.mockResolvedValue({ isPastDue: true, isOwner: true });

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );

  expect(response.status).toBe(401);
  expect(findMany).not.toHaveBeenCalled();
  expect(rawQuery).not.toHaveBeenCalled();
});

test('allows an admin personal workspace without applying the past-due gate', async () => {
  parsed(
    {
      websiteIds: ids,
      startAt: Date.UTC(2026, 10, 1),
      endAt: Date.UTC(2026, 10, 20),
      testMode: false,
    },
    { user: { id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', isAdmin: true } },
  );
  billingAccess.mockResolvedValue({ isPastDue: true, isOwner: true });

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );

  expect(response.status).toBe(200);
  expect(billingAccess).not.toHaveBeenCalled();
});

test('clamps personal Cloud metrics to the six-month subscription floor', async () => {
  vi.stubEnv('CLOUD_MODE', '1');
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-20T12:00:00.000Z'));
  fetchAccountMock.mockResolvedValue({ hasSubscription: false } as any);
  parsed({
    websiteIds: ids,
    startAt: Date.UTC(2026, 0, 1),
    endAt: Date.UTC(2026, 10, 20),
    testMode: false,
  });

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );
  const [, params] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(params).toMatchObject({
    startDate: new Date('2026-05-01T07:00:00.000Z'),
  });
});

test('returns zero personal metrics without querying for a wholly pre-floor range', async () => {
  vi.stubEnv('CLOUD_MODE', '1');
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-20T12:00:00.000Z'));
  fetchAccountMock.mockResolvedValue({ hasSubscription: false } as any);
  parsed({
    websiteIds: ids,
    startAt: Date.UTC(2026, 0, 1),
    endAt: Date.UTC(2026, 3, 30),
    testMode: false,
  });

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );

  expect(response.status).toBe(200);
  await expect(response.json()).resolves.toEqual({
    data: ids.map(websiteId => ({ websiteId, users: 0, errors: 0 })),
  });
  expect(rawQuery).not.toHaveBeenCalled();
});

test('team workspace verifies membership and scopes every website to that team', async () => {
  const teamId = '33333333-3333-4333-8333-333333333333';
  parsed({
    websiteIds: ids,
    startAt: Date.UTC(2026, 10, 1),
    endAt: Date.UTC(2026, 10, 20),
    testMode: true,
    teamId,
  });
  vi.mocked(canViewTeam).mockResolvedValue(true);

  const response = await POST(
    new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
  );

  expect(response.status).toBe(200);
  expect(canViewTeam).toHaveBeenCalledWith(expect.anything(), teamId);
  expect(billingAccess).not.toHaveBeenCalled();
  expect(findMany).toHaveBeenCalledWith(
    expect.objectContaining({
      where: expect.objectContaining({ teamId }),
    }),
  );
});

describe('fail-closed workspace access', () => {
  test('rejects share credentials even if a user credential is also present', async () => {
    parsed(
      {
        websiteIds: ids,
        startAt: Date.UTC(2026, 10, 1),
        endAt: Date.UTC(2026, 10, 20),
        testMode: false,
      },
      { shareToken: { websiteIds: ids } },
    );

    const response = await POST(
      new Request('http://localhost/api/websites/card-metrics', {
        method: 'POST',
        headers: { 'x-umami-share-token': 'also-present' },
      }),
    );

    expect(response.status).toBe(401);
    expect(findMany).not.toHaveBeenCalled();
    expect(rawQuery).not.toHaveBeenCalled();
  });

  test('rejects the whole batch when any website is outside the selected workspace', async () => {
    findMany.mockResolvedValue([{ id: ids[0], domain: 'app.ios' }] as any);

    const response = await POST(
      new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
    );

    expect(response.status).toBe(401);
    expect(rawQuery).not.toHaveBeenCalled();
  });

  test('rejects a team batch before loading websites when the caller cannot view the team', async () => {
    const teamId = '33333333-3333-4333-8333-333333333333';
    parsed({
      websiteIds: ids,
      startAt: Date.UTC(2026, 10, 1),
      endAt: Date.UTC(2026, 10, 20),
      testMode: false,
      teamId,
    });
    vi.mocked(canViewTeam).mockResolvedValue(false);

    const response = await POST(
      new Request('http://localhost/api/websites/card-metrics', { method: 'POST' }),
    );

    expect(response.status).toBe(401);
    expect(findMany).not.toHaveBeenCalled();
  });
});
