import { afterEach, beforeEach, expect, test, vi } from 'vitest';
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
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-20T12:00:00.000Z'));
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: {},
    error: undefined,
  } as any);
  vi.mocked(canViewWebsite).mockResolvedValue(true);
  vi.mocked(getQueryFilters).mockResolvedValue({
    minimumDate: new Date('2026-11-20T11:30:00.000Z'),
    startDate: new Date('2026-11-20T11:30:00.000Z'),
    endDate: new Date('2026-11-21T07:59:59.999Z'),
    timezone: 'America/Los_Angeles',
    tag: 'neq.test',
  } as any);
  rawQuery.mockImplementation(async (_sql, _params, name) => {
    if (name === 'getAppOverviewSummary') {
      return [{ events: 8n, totalUsers: 2n, usersLastHour: 1n }];
    }
    if (name === 'getAppOverviewDailyUsers') {
      return [{ x: '2026-11-20', y: 2n }];
    }
    if (name === 'getAppOverviewDimensions') {
      return [
        { dimension: 'sdkName', value: 'TelemetryDeckSwift', users: 2n },
        { dimension: 'sdkVersion', value: '2.5.0', users: 2n },
        { dimension: 'appVersion', value: '1.0', users: 2n },
        { dimension: 'buildNumber', value: '100', users: 2n },
        { dimension: 'systemVersion', value: 'iOS 18.4.1', users: 2n },
      ];
    }
    return [{ day: 1, eligibleUsers: 2n, returningUsers: 1n, percentage: '50.0' }];
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

test('uses an actual rolling hour and returns distinct SDK identity metrics', async () => {
  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/overview',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );
  const body = await response.json();
  const summaryCall = rawQuery.mock.calls.find(([, , name]) => name === 'getAppOverviewSummary');

  expect(response.status).toBe(200);
  expect(summaryCall?.[1]).toMatchObject({
    lastHourStart: new Date('2026-11-20T11:30:00.000Z'),
    now: new Date('2026-11-20T12:00:00.000Z'),
  });
  const retentionCall = rawQuery.mock.calls.find(
    ([, , name]) => name === 'getAppOverviewRetention',
  );
  expect(retentionCall?.[0]).toContain('e.created_at >= {{minimumDate}}');
  expect(retentionCall?.[1]).toMatchObject({
    minimumDate: new Date('2026-11-20T11:30:00.000Z'),
  });
  expect(body.identity).toBe('clientUser');
  expect(body.summary).toEqual({
    users: 2,
    usersLastHour: 1,
    events: 8,
    errors: null,
    mostUsedSdk: 'TelemetryDeckSwift',
  });
});

test('does not expose app identity dimensions through a share token', async () => {
  vi.mocked(parseRequest).mockResolvedValue({
    auth: { shareToken: { websiteId: '00000000-0000-0000-0000-000000000001' } },
    query: {},
    error: undefined,
  } as any);

  const response = await GET(
    new Request(
      'http://localhost/api/websites/00000000-0000-0000-0000-000000000001/app-reports/overview',
    ),
    { params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }) },
  );

  expect(response.status).toBe(401);
  expect(canViewWebsite).not.toHaveBeenCalled();
  expect(rawQuery).not.toHaveBeenCalled();
});
