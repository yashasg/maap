import { beforeEach, expect, test, vi } from 'vitest';
import { SHARE_CONTEXT_HEADER } from '@/lib/constants';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { canViewWebsite } from '@/permissions';
import { GET } from './route';

vi.mock('@/lib/prisma', () => ({
  default: { rawQuery: vi.fn() },
}));

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
  getQueryFilters: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canViewWebsite: vi.fn(),
}));

const rawQuery = vi.mocked(prisma.rawQuery);

beforeEach(() => {
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: { testMode: 'true' },
    error: undefined,
  } as any);
  vi.mocked(canViewWebsite).mockReset().mockResolvedValue(true);
  vi.mocked(getQueryFilters).mockResolvedValue({
    minimumDate: new Date('2026-06-01T00:00:00.000Z'),
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-20T23:59:59.999Z'),
    timezone: 'UTC',
  } as any);
  rawQuery.mockReset();
});

test('returns one activated SDK user despite repeated qualifying signals', async () => {
  rawQuery.mockResolvedValue([
    { section: 'summary', metric: 'totalSignals', bucket: '', users: 6n },
    { section: 'summary', metric: 'identifiedSignals', bucket: '', users: 6n },
    { section: 'summary', metric: 'inputSignals', bucket: '', users: 6n },
    { section: 'summary', metric: 'activatedUsers', bucket: '', users: 1n },
    { section: 'availability', metric: 'hasTotalSessions', bucket: '', users: 6n },
    { section: 'availability', metric: 'hasAverageSeconds', bucket: '', users: 6n },
    { section: 'series', metric: 'daily', bucket: '2026-11-20', users: 1n },
  ]);

  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/activation?startAt=1&endAt=2&testMode=true',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );
  const body = await response.json();
  const [sql, params] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(body.scope.activatedUsers).toBe(1);
  expect(body.scope.testMode).toBe(true);
  expect(body.definition).toMatchObject({ thresholdSeconds: 300, sessionLimit: 5 });
  expect(body.series.daily).toEqual([{ key: '2026-11-20', label: '2026-11-20', count: 1 }]);
  expect(sql).toContain('distinct on (user_id)');
  expect(sql).toContain('accumulated_seconds >= 300');
  expect(sql).toContain("e.tag = 'test'");
  expect(sql).toContain('e.created_at >= {{minimumDate}}');
  expect(params).toMatchObject({
    websiteId: '00000000-0000-0000-0000-000000000001',
    minimumDate: new Date('2026-06-01T00:00:00.000Z'),
    timezone: 'UTC',
  });
});

test('returns an honest configuration path when activation fields were discarded', async () => {
  rawQuery.mockResolvedValue([
    { section: 'summary', metric: 'totalSignals', bucket: '', users: 3n },
    { section: 'summary', metric: 'identifiedSignals', bucket: '', users: 3n },
    { section: 'summary', metric: 'inputSignals', bucket: '', users: 0n },
    { section: 'summary', metric: 'activatedUsers', bucket: '', users: 0n },
  ]);

  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/activation?startAt=1&endAt=2',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );
  const body = await response.json();

  expect(body.configuration.status).toBe('configuration-missing');
  expect(body.configuration.environmentVariable).toBe('TD_ALLOWED_KEYS');
  expect(body.configuration.missingKeys).toContain('TelemetryDeck.Retention.totalSessionsCount');
  expect(body.scope.activatedUsers).toBe(0);
});

test('does not query activation data without app access', async () => {
  vi.mocked(canViewWebsite).mockResolvedValue(false);

  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/activation?startAt=1&endAt=2',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );

  expect(response.status).toBe(401);
  expect(rawQuery).not.toHaveBeenCalled();
});

test('requires a user for Activation even when the share grants Overview', async () => {
  const websiteId = '00000000-0000-0000-0000-000000000001';
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { shareToken: { websiteId, parameters: { overview: true, events: false } } },
    query: {},
  } as Awaited<ReturnType<typeof parseRequest>>);
  const response = await GET(new Request('http://localhost'), {
    params: Promise.resolve({ websiteId }),
  });
  expect(response.status).toBe(401);
  expect(canViewWebsite).not.toHaveBeenCalled();
  expect(rawQuery).not.toHaveBeenCalled();
});

test('does not use a signed-in session to expose Activation in a share context', async () => {
  const response = await GET(
    new Request('http://localhost', { headers: { [SHARE_CONTEXT_HEADER]: '1' } }),
    {
      params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }),
    },
  );
  expect(response.status).toBe(401);
  expect(canViewWebsite).not.toHaveBeenCalled();
  expect(rawQuery).not.toHaveBeenCalled();
});
