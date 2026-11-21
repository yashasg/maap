import { beforeEach, expect, test, vi } from 'vitest';
import prisma from '@/lib/prisma';
import { getQueryFilters, parseRequest } from '@/lib/request';
import { canViewWebsiteSection } from '@/permissions';
import { GET, versionsQuerySchema } from './route';

vi.mock('@/lib/prisma', () => ({
  default: { rawQuery: vi.fn() },
}));

vi.mock('@/lib/request', () => ({
  parseRequest: vi.fn(),
  getQueryFilters: vi.fn(),
}));

vi.mock('@/permissions', () => ({
  canViewWebsiteSection: vi.fn(),
}));

const rawQuery = vi.mocked(prisma.rawQuery);
const parseRequestMock = vi.mocked(parseRequest);
const getQueryFiltersMock = vi.mocked(getQueryFilters);
const canViewWebsiteSectionMock = vi.mocked(canViewWebsiteSection);

const context = {
  params: Promise.resolve({ websiteId: '00000000-0000-0000-0000-000000000001' }),
};

beforeEach(() => {
  vi.clearAllMocks();
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: { dimension: 'appVersion' },
    error: undefined,
  } as any);
  canViewWebsiteSectionMock.mockResolvedValue(true);
  getQueryFiltersMock.mockResolvedValue({
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-30T23:59:59.999Z'),
    tag: 'neq.test',
  } as any);
  rawQuery.mockResolvedValue([
    { value: '2.0', events: 7n },
    { value: null, events: 3n },
  ]);
});

test('validates the version dimension and Test Mode parameters', () => {
  expect(
    versionsQuerySchema.safeParse({
      dimension: 'sessionBrowser',
      startAt: 1,
      endAt: 2,
    }).success,
  ).toBe(false);
  expect(
    versionsQuerySchema.safeParse({
      dimension: 'appVersion',
      startAt: 1,
      endAt: 2,
      testMode: 'maybe',
    }).success,
  ).toBe(false);
  expect(
    versionsQuerySchema.safeParse({
      dimension: 'appVersion',
      startAt: 1,
      endAt: 2,
      testMode: 'true',
      tag: 'test',
    }).success,
  ).toBe(false);
  expect(versionsQuerySchema.safeParse({ dimension: 'appVersion' }).success).toBe(false);
});

test('aggregates the selected property on every scoped signal', async () => {
  const response = await GET(
    new Request('http://localhost/api/websites/site/app-reports/versions'),
    context,
  );
  const body = await response.json();
  const [sql, parameters, name] = rawQuery.mock.calls[0];

  expect(response.status).toBe(200);
  expect(parseRequestMock).toHaveBeenCalledWith(expect.any(Request), versionsQuerySchema);
  expect(getQueryFiltersMock).toHaveBeenCalledWith(
    { dimension: 'appVersion' },
    '00000000-0000-0000-0000-000000000001',
  );
  expect(canViewWebsiteSectionMock).toHaveBeenCalledWith(
    { user: { id: 'user-1' } },
    '00000000-0000-0000-0000-000000000001',
    'events',
  );
  expect(sql).toContain('from website_event e');
  expect(sql).toContain('from event_data d');
  expect(sql).not.toContain('join session');
  expect(sql).toContain("coalesce(e.tag, '') != 'test'");
  expect(parameters).toMatchObject({
    primaryKey: 'TelemetryDeck.AppInfo.version',
    fallbackKey: 'appVersion',
  });
  expect(name).toBe('getAppVersions');
  expect(body).toEqual({
    dimension: 'appVersion',
    totalEvents: 10,
    reportedEvents: 7,
    values: [
      { value: '2.0', events: 7 },
      { value: null, events: 3 },
    ],
  });
});

test('selects SDK properties and test-tagged signals in Test Mode', async () => {
  parseRequestMock.mockResolvedValue({
    auth: { user: { id: 'user-1' } },
    query: { dimension: 'sdkVersion', testMode: 'true' },
    error: undefined,
  } as any);
  getQueryFiltersMock.mockResolvedValue({
    startDate: new Date('2026-11-01T00:00:00.000Z'),
    endDate: new Date('2026-11-30T23:59:59.999Z'),
    tag: 'test',
  } as any);

  await GET(new Request('http://localhost/api/websites/site/app-reports/versions'), context);

  expect(rawQuery.mock.calls[0][0]).toContain("e.tag = 'test'");
  expect(rawQuery.mock.calls[0][1]).toMatchObject({
    primaryKey: 'TelemetryDeck.SDK.version',
    fallbackKey: 'sdkVersion',
  });
});

test('rejects unauthorized access before querying signal data', async () => {
  canViewWebsiteSectionMock.mockResolvedValue(false);

  const response = await GET(
    new Request('http://localhost/api/websites/site/app-reports/versions'),
    context,
  );

  expect(response.status).toBe(401);
  expect(rawQuery).not.toHaveBeenCalled();
});

test('rejects a reversed date range', async () => {
  getQueryFiltersMock.mockResolvedValue({
    startDate: new Date('2026-12-01T00:00:00.000Z'),
    endDate: new Date('2026-11-01T00:00:00.000Z'),
    tag: 'neq.test',
  } as any);

  const response = await GET(
    new Request('http://localhost/api/websites/site/app-reports/versions'),
    context,
  );

  expect(response.status).toBe(400);
  expect(rawQuery).not.toHaveBeenCalled();
});
