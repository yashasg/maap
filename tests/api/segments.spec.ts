import { expect, test } from './fixtures';
import { UNKNOWN_UUID } from './helpers/constants';
import { dateRange } from './helpers/dates';
import { createWebsite, deleteWebsite, uniqueName } from './helpers/entities';

test.describe('Segments', () => {
  test.describe.configure({ mode: 'serial' });

  const parameters = {
    filters: [{ name: 'path', operator: 'eq', value: '/pricing' }],
    match: 'all',
  };

  let websiteId = '';
  let segmentId = '';
  let cohortId = '';
  let seedSegmentId = '';
  let base = '';

  test.beforeAll(async ({ admin }) => {
    websiteId = (await createWebsite(admin)).id;
    base = `/api/websites/${websiteId}/segments`;
  });

  test.afterAll(async ({ admin, seed }) => {
    if (seedSegmentId) {
      await admin.del(`/api/websites/${seed.website.id}/segments/${seedSegmentId}`);
    }

    if (websiteId) {
      await deleteWebsite(admin, websiteId);
    }
  });

  test('POST /api/websites/{websiteId}/segments creates a segment', async ({ admin }) => {
    const name = uniqueName('segment');
    const response = await admin.post(base, { type: 'segment', name, parameters });

    expect(response.status).toBe(200);
    expect(response.body).toMatchObject({
      id: expect.any(String),
      websiteId,
      type: 'segment',
      name,
      parameters,
      createdAt: expect.any(String),
    });

    segmentId = response.body.id;
  });

  test('POST /api/websites/{websiteId}/segments creates a cohort', async ({ admin }) => {
    const name = uniqueName('cohort');
    const response = await admin.post(base, {
      type: 'cohort',
      name,
      parameters: { ...parameters, dateRange: '30d', action: { type: 'event', value: 'signup' } },
    });

    expect(response.status).toBe(200);
    expect(response.body).toMatchObject({ websiteId, type: 'cohort', name });

    cohortId = response.body.id;
  });

  test('POST /api/websites/{websiteId}/segments validates and requires permission', async ({
    admin,
    user,
    viewer,
    api,
  }) => {
    const badType = await admin.post(base, { type: 'nope', name: 'x', parameters });
    const missingParams = await admin.post(base, { type: 'segment', name: 'x' });
    const badOperator = await admin.post(base, {
      type: 'segment',
      name: 'x',
      parameters: { filters: [{ name: 'path', operator: 'nope', value: '/' }] },
    });
    const tooLong = await admin.post(base, { type: 'segment', name: 'x'.repeat(201), parameters });
    const notOwner = await user.post(base, { type: 'segment', name: 'x', parameters });
    const denied = await viewer.post(base, { type: 'segment', name: 'x', parameters });
    const anonymous = await api.post(base, { type: 'segment', name: 'x', parameters });

    expect(badType.status).toBe(400);
    expect(missingParams.status).toBe(400);
    expect(badOperator.status).toBe(400);
    expect(tooLong.status).toBe(400);
    expect(notOwner.status).toBe(401);
    expect(denied.status).toBe(401);
    expect(anonymous.status).toBe(401);
  });

  test('GET /api/websites/{websiteId}/segments lists segments by type', async ({
    admin,
    viewer,
    share,
    seed,
  }) => {
    const segments = await admin.get(base, { params: { type: 'segment' } });
    const cohorts = await admin.get(base, { params: { type: 'cohort' } });
    const search = await admin.get(base, {
      params: { type: 'segment', search: 'zzz-no-such-segment' },
    });
    const missingType = await admin.get(base);
    const badType = await admin.get(base, { params: { type: 'nope' } });
    const denied = await viewer.get(base, { params: { type: 'segment' } });
    const shared = await (await share()).get(`/api/websites/${seed.website.id}/segments`, {
      params: { type: 'segment' },
    });

    expect(segments.status).toBe(200);
    expect(segments.body).toMatchObject({
      data: expect.any(Array),
      count: 1,
      page: 1,
      pageSize: expect.any(Number),
    });
    expect(segments.body.data[0]).toMatchObject({ id: segmentId, type: 'segment', parameters });
    expect(cohorts.body.data.map((s: any) => s.id)).toEqual([cohortId]);
    expect(search.body.data).toHaveLength(0);
    expect(missingType.status).toBe(400);
    expect(badType.status).toBe(400);
    expect(denied.status).toBe(401);
    expect(shared.status).toBe(200);
  });

  test('GET /api/websites/{websiteId}/segments/{segmentId} returns a segment', async ({
    admin,
    viewer,
  }) => {
    const response = await admin.get(`${base}/${segmentId}`);
    const denied = await viewer.get(`${base}/${segmentId}`);
    const unknown = await admin.get(`${base}/${UNKNOWN_UUID}`);

    expect(response.status).toBe(200);
    expect(response.body).toMatchObject({ id: segmentId, websiteId, type: 'segment', parameters });
    expect(denied.status).toBe(401);
    expect(unknown.status).toBe(404);
  });

  test('POST /api/websites/{websiteId}/segments/{segmentId} updates a segment', async ({
    admin,
    user,
  }) => {
    const name = uniqueName('renamed');
    const updated = { filters: [{ name: 'path', operator: 'c', value: '/docs' }], match: 'any' };
    const response = await admin.post(`${base}/${segmentId}`, {
      type: 'segment',
      name,
      parameters: updated,
    });
    const invalid = await admin.post(`${base}/${segmentId}`, { name });
    const unknown = await admin.post(`${base}/${UNKNOWN_UUID}`, {
      type: 'segment',
      name,
      parameters: updated,
    });
    const denied = await user.post(`${base}/${segmentId}`, {
      type: 'segment',
      name,
      parameters: updated,
    });

    expect(response.status).toBe(200);
    expect(response.body).toMatchObject({ id: segmentId, name, parameters: updated });
    expect(invalid.status).toBe(400);
    expect(unknown.status).toBe(404);
    expect(denied.status).toBe(401);
  });

  test('GET /api/websites/{websiteId}/stats accepts a segment filter', async ({ admin, seed }) => {
    const segmentParameters = {
      filters: [{ name: 'distinctId', operator: 'eq', value: seed.data.distinctIds[0] }],
      match: 'all',
    };
    const created = await admin.post(`/api/websites/${seed.website.id}/segments`, {
      type: 'segment',
      name: uniqueName('segment'),
      parameters: segmentParameters,
    });

    expect(created.status).toBe(200);
    seedSegmentId = created.body.id;

    const path = `/api/websites/${seed.website.id}/stats`;
    const unfiltered = await admin.get(path, {
      params: dateRange(seed),
    });
    const stats = await admin.get(path, {
      params: dateRange(seed, { segment: seedSegmentId }),
    });
    const direct = await admin.get(path, {
      params: dateRange(seed, {
        distinctId: seed.data.distinctIds[0],
      }),
    });

    expect(unfiltered.status).toBe(200);
    expect(stats.status).toBe(200);
    expect(direct.status).toBe(200);
    expect(Number(stats.body.pageviews)).toBeGreaterThan(0);
    expect(Number(stats.body.pageviews)).toBe(Number(direct.body.pageviews));
    expect(Number(stats.body.pageviews)).toBeLessThan(Number(unfiltered.body.pageviews));
  });

  test('cohort membership and typed properties intersect on the same purchase', async ({
    admin,
    viewer,
    seed,
  }) => {
    const base = `/api/websites/${seed.website.id}`;
    const range = dateRange(seed);
    const parameters = {
      dateRange: `range:${range.startAt}:${range.endAt}`,
      action: { type: 'event', value: 'purchase' },
      filters: [{ name: 'distinctId', operator: 'eq', value: seed.data.distinctIds[0] }],
      match: 'all',
    };
    const created = await admin.post(`${base}/segments`, {
      type: 'cohort',
      name: uniqueName('buyers'),
      parameters,
    });
    expect(created.status).toBe(200);
    const cohort = created.body.id;
    try {
      const denied = await viewer.get(`${base}/stats`, { params: { ...range, cohort } });
      expect(denied.status).toBe(401);
      const total = async (filters: Record<string, string>) => {
        const response = await admin.get(`${base}/event-data-pivot/numeric-stats`, {
          params: { ...range, eventName: 'purchase', propertyName: 'revenue', ...filters },
        });
        expect(response.status, response.text).toBe(200);
        return Number(response.body.total);
      };
      expect(await total({ cohort })).toBe(3 * 49.5);
      expect(await total({ cohort, epf0: '5.c.items.sku-1', spf0: '2.eq.seats.5' })).toBe(3 * 49.5);
      const yesterday = new Date(seed.range.endAt - 86_400_000).toISOString().slice(0, 10);
      expect(await total({ cohort, epf0: `4.eq.orderedAt.${yesterday}` })).toBe(49.5);
      expect(await total({ cohort, epf0: '2.gt.revenue.50' })).toBe(0);
      expect(await total({ cohort, spf0: '2.eq.seats.6' })).toBe(0);
      const filters = { ...range, cohort, epf0: '1.eq.currency.USD', spf0: '2.eq.seats.5' };
      for (const [endpoint, count] of [
        ['events', 3],
        ['sessions', 1],
        ['event-data', 3],
      ] as const) {
        const page = await admin.get(`${base}/${endpoint}`, {
          params: { ...filters, pageSize: 1 },
        });
        expect(page.status, page.text).toBe(200);
        expect(page.body.count, endpoint).toBe(count);
        expect(page.body.data).toHaveLength(1);
      }
      const heatmap = await admin.get(`${base}/heatmaps`, {
        params: { ...range, mode: 'click', urlPath: '/', cohort },
      });
      expect(heatmap.status, heatmap.text).toBe(200);
      expect(heatmap.body.points.length).toBeGreaterThan(0);
      const excluded = await admin.get(`${base}/heatmaps`, {
        params: { ...range, mode: 'click', urlPath: '/', cohort, spf0: '2.eq.seats.6' },
      });
      expect(excluded.status, excluded.text).toBe(200);
      expect(excluded.body.pages).toEqual([]);
      expect(excluded.body.points).toEqual([]);

      const updated = await admin.post(`${base}/segments/${cohort}`, {
        type: 'cohort',
        name: uniqueName('past buyers'),
        parameters: { ...parameters, dateRange: 'range:1000:2000' },
      });
      expect(updated.status).toBe(200);
      expect(await total({ cohort })).toBe(0);
    } finally {
      expect((await admin.del(`${base}/segments/${cohort}`)).status).toBe(200);
    }
  });

  test('saved session properties intersect with request properties instead of replacing them', async ({
    admin,
    seed,
  }) => {
    const base = `/api/websites/${seed.website.id}`;
    const saved = await admin.post(`${base}/segments`, {
      type: 'segment',
      name: uniqueName('five seats'),
      parameters: {
        sessionPropertyFilters: [
          { propertyName: 'seats', operator: 'eq', dataType: 2, value: '5' },
        ],
      },
    });
    expect(saved.status).toBe(200);
    try {
      for (const [plan, expected] of [
        ['pro', 5],
        ['free', 0],
      ] as const) {
        const stats = await admin.get(`${base}/session-data/numeric-stats`, {
          params: dateRange(seed, {
            propertyName: 'seats',
            segment: saved.body.id,
            spf0: `1.eq.plan.${plan}`,
          }),
        });
        expect(stats.status, stats.text).toBe(200);
        expect(Number(stats.body.total)).toBe(expected);
      }
    } finally {
      expect((await admin.del(`${base}/segments/${saved.body.id}`)).status).toBe(200);
    }
  });

  test('DELETE /api/websites/{websiteId}/segments/{segmentId} deletes a segment', async ({
    admin,
    user,
  }) => {
    const denied = await user.del(`${base}/${segmentId}`);
    const response = await admin.del(`${base}/${segmentId}`);
    const cohort = await admin.del(`${base}/${cohortId}`);
    const gone = await admin.get(`${base}/${segmentId}`);
    const unknown = await admin.del(`${base}/${UNKNOWN_UUID}`);

    expect(denied.status).toBe(401);
    expect(response.status).toBe(200);
    expect(response.body).toEqual({ ok: true });
    expect(cohort.status).toBe(200);
    expect(gone.status).toBe(404);
    expect(unknown.status).toBe(404);

    segmentId = '';
    cohortId = '';
  });
});
