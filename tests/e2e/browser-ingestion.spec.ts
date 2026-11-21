import { randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { expect, test } from '@playwright/test';
import { authHeaders, deleteWebsite, loginViaApi } from './helpers';

test('cross-origin browser ingestion persists events and exposes errors', async ({
  page,
  request,
  baseURL,
}) => {
  if (!baseURL) throw new Error('Missing collector base URL');
  const auth = await loginViaApi(request);
  const website = randomUUID();
  const created = await request.post('/api/websites', {
    headers: authHeaders(auth),
    data: { id: website, name: 'Browser ingestion', domain: 'localhost' },
  });
  expect(created.status()).toBe(200);

  // A real second origin lets Chromium enforce CORS, including its own preflights.
  const server = createServer((_req, res) => {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end('<!doctype html><title>Browser ingestion</title>');
  });
  try {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('Missing browser origin port');
    const browserOrigin = `http://localhost:${address.port}`;
    expect(browserOrigin).not.toBe(new URL(baseURL).origin);
    await page.goto(browserOrigin);

    const startAt = Date.now() - 60_000;
    const paths = [
      '/api/send',
      '/api/batch',
      '/v2',
      '/v2/',
      '/v2/namespace/browser',
      '/v2/namespace/browser/',
    ];
    const expectedPaths: string[] = [];
    let cache = '';
    for (const [index, path] of paths.entries()) {
      const url = new URL(path, baseURL).href;
      const eventPath = `/browser-ingestion/${index}`;
      const event = {
        type: 'event',
        payload: { website, hostname: 'localhost', url: eventPath },
      };
      const body = path.startsWith('/v2')
        ? [
            {
              appID: website,
              clientUser: 'b'.repeat(64),
              type: 'TelemetryDeck.Navigation.pathChanged',
              payload: { 'TelemetryDeck.Navigation.destinationPath': eventPath },
            },
            { appID: 'invalid', type: 'InvalidSignal' },
          ]
        : path === '/api/batch'
          ? [event, { ...event, payload: { ...event.payload, website: randomUUID() } }]
          : event;

      const preflight = await request.fetch(url, {
        method: 'OPTIONS',
        maxRedirects: 0,
        headers: {
          Origin: browserOrigin,
          'Access-Control-Request-Method': 'POST',
          'Access-Control-Request-Headers': 'content-type,x-umami-cache',
        },
      });
      expect(preflight.status(), `${path} preflight must not redirect`).toBe(204);
      expect(preflight.headers()['access-control-allow-origin']).toBe('*');
      expect(preflight.headers()['access-control-allow-methods']).toContain('POST');

      const response = await page.evaluate(
        async ({ url, body, cache }) => {
          const response = await fetch(url, {
            method: 'POST',
            credentials: 'omit',
            headers: {
              'Content-Type': 'application/json',
              ...(cache ? { 'x-umami-cache': cache } : {}),
            },
            body: JSON.stringify(body),
          });
          return { status: response.status, body: await response.json() };
        },
        { url, body, cache },
      );
      expect(response.status, path).toBe(200);
      if (path.startsWith('/v2')) {
        expect(response.body).toEqual({ accepted: 1, rejected: 1, dropped: 0 });
      } else if (path === '/api/batch') {
        expect(response.body).toMatchObject({ size: 2, processed: 1, errors: 1 });
        expect(response.body.details[0].index).toBe(1);
      } else {
        expect(response.body).toMatchObject({
          cache: expect.any(String),
          sessionId: expect.any(String),
          visitId: expect.any(String),
        });
        cache = response.body.cache;
        const repeated = await page.evaluate(
          async ({ url, event, cache }) => {
            const response = await fetch(url, {
              method: 'POST',
              credentials: 'omit',
              headers: { 'Content-Type': 'application/json', 'x-umami-cache': cache },
              body: JSON.stringify(event),
            });
            return { status: response.status, body: await response.json() };
          },
          {
            url,
            event: { ...event, payload: { ...event.payload, url: '/browser-ingestion/repeat' } },
            cache,
          },
        );
        expect(repeated.status).toBe(200);
        expect(repeated.body.sessionId).toBe(response.body.sessionId);
        expect(repeated.body.visitId).toBe(response.body.visitId);
        expectedPaths.push('/browser-ingestion/repeat');
      }
      expectedPaths.push(eventPath);

      const invalid = await page.evaluate(async url => {
        const response = await fetch(url, {
          method: 'POST',
          credentials: 'omit',
          headers: { 'Content-Type': 'application/json' },
          body: '[',
        });
        return { status: response.status, body: await response.text() };
      }, url);
      expect(invalid.status, `${path} invalid body`).toBe(400);
      expect(invalid.body).not.toBe('');
    }

    const metricsPath = `/api/websites/${website}/metrics`;
    const params = { startAt, endAt: Date.now() + 1000, type: 'path' };
    const metrics = await request.get(metricsPath, { headers: authHeaders(auth), params });
    expect(metrics.status()).toBe(200);
    expect(await metrics.json()).toEqual(
      expect.arrayContaining(expectedPaths.map(x => ({ x, y: 1 }))),
    );
    expect(await metrics.json()).toHaveLength(expectedPaths.length);

    // New documents have neither the in-memory tracker cache nor the SDK's old clientUser.
    for (let reload = 0; reload < 5; reload++) {
      await page.reload();
      const result = await page.evaluate(
        async ({ collector, website, reload }) => {
          const response = await fetch(`${collector}/v2/`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify([
              {
                appID: website,
                clientUser: String(reload).repeat(64),
                type: 'TelemetryDeck.Navigation.pathChanged',
                payload: { 'TelemetryDeck.Navigation.destinationPath': '/reload' },
              },
            ]),
          });
          return response.json();
        },
        { collector: baseURL, website, reload },
      );
      expect(result).toEqual({ accepted: 1, rejected: 0, dropped: 0 });
    }
    const statsUrl = `/api/websites/${website}/stats`;
    const statsParams = { startAt, endAt: Date.now() + 1000 };
    const before = await (
      await request.get(statsUrl, { headers: authHeaders(auth), params: statsParams })
    ).json();
    expect(Number(before.visitors)).toBe(1);
    expect(Number(before.pageviews)).toBe(expectedPaths.length + 5);

    // A cached website token must not authorize a different browser origin.
    for (const url of [`/console/${website}`, '/websites', '/']) {
      const response = await request.post('/api/send', {
        headers: { origin: 'https://maap.kashiworks.com', 'x-umami-cache': cache },
        data: { type: 'event', payload: { website, hostname: 'maap.kashiworks.com', url } },
      });
      expect(response.status()).toBe(403);
    }
    const after = await (
      await request.get(statsUrl, {
        headers: authHeaders(auth),
        params: { ...statsParams, endAt: Date.now() + 1000 },
      })
    ).json();
    expect(after.visitors).toBe(before.visitors);
    expect(after.pageviews).toBe(before.pageviews);

    const unauthorized = await request.get(metricsPath, { params });
    expect(unauthorized.status()).toBe(401);
  } finally {
    if (server.listening) {
      await new Promise<void>((resolve, reject) =>
        server.close(error => (error ? reject(error) : resolve())),
      );
    }
    await deleteWebsite(request, auth, website);
  }
});
