import { expect, test } from './fixtures';
import { createWebsite, deleteWebsite } from './helpers/entities';

test.describe('App acquisition report', () => {
  let websiteId = '';

  test.beforeAll(async ({ admin }) => {
    const website = await createWebsite(admin, { name: 'Acquisition fixture' });
    websiteId = website.id;
  });

  test.afterAll(async ({ admin }) => {
    if (websiteId) {
      await deleteWebsite(admin, websiteId);
    }
  });

  test('uses historical SDK first-seen identity instead of sessions', async ({ api, admin }) => {
    const now = Date.now();
    const historical = new Date(now - 2 * 24 * 60 * 60 * 1000).toISOString();
    const existingUser = 'a'.repeat(64);
    const newUser = 'b'.repeat(64);
    const base = {
      appID: websiteId,
      isTestMode: 'false',
      payload: {
        'TelemetryDeck.Device.operatingSystem': 'iOS',
        'TelemetryDeck.Device.modelName': 'iPhone17,1',
      },
    };

    const historicalResponse = await api.post('/v2/', [
      {
        ...base,
        clientUser: existingUser,
        type: 'fixture.first',
        receivedAt: historical,
      },
    ]);
    expect(historicalResponse.status).toBe(200);

    const activeResponse = await api.post('/v2/', [
      { ...base, clientUser: existingUser, type: 'fixture.returned' },
      { ...base, clientUser: newUser, type: 'fixture.new' },
    ]);
    expect(activeResponse.status).toBe(200);

    const report = await admin.get(`/api/websites/${websiteId}/app-reports/acquisition`, {
      params: {
        startAt: now - 60_000,
        endAt: now + 60_000,
        timezone: 'UTC',
      },
    });

    expect(report.status).toBe(200);
    expect(report.body.identity).toMatchObject({
      unit: 'appUser',
      source: 'clientUser',
      unidentifiedEventsExcluded: true,
    });
    expect(report.body.range).toMatchObject({
      totalSignals: 2,
      identifiedSignals: 2,
    });
    expect(report.body.series.daily).toEqual([
      expect.objectContaining({ activeUsers: 2, newUsers: 1 }),
    ]);
    expect(report.body.query.grains).toMatchObject({
      hourly: { unit: 'hour', window: 'last24Hours' },
      daily: { unit: 'day', window: 'selectedRange' },
      weekly: { unit: 'week', window: 'last3CalendarMonths' },
      monthly: { unit: 'month', window: 'last12CalendarMonths' },
    });
    expect(report.body.query.milliseconds).toEqual(expect.any(Number));
  });
});
