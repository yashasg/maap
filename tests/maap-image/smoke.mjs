// Run inside the disposable CI Umami container; never point this at a deployed instance.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';

const origin = process.env.MAAP_SMOKE_ORIGIN || 'http://127.0.0.1:3000';
const unmatched = await fetch(`${origin}/api/maap-smoke-unmatched`, { redirect: 'manual' });
assert.equal(unmatched.status, 404);
assert.equal(unmatched.headers.get('x-maap-server'), 'rust', 'Unmatched routes must not fall through to Next.js');

async function request(path, { body, token, headers } = {}) {
  const res = await fetch(origin + path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: {
      'content-type': 'application/json',
      ...(token && { authorization: `Bearer ${token}` }),
      ...headers,
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(30000),
  });
  assert.equal(new URL(res.url).origin, origin);
  assert.equal(res.status, 200, `${path}: HTTP ${res.status}`);
  return res;
}

const { token, user } = await (
  await request('/api/auth/login', { body: { username: 'admin', password: 'umami' } })
).json();
assert.ok(token);
assert.ok(user?.id);
const appID = randomUUID();
await request('/api/websites', {
  token,
  body: {
    id: appID,
    name: 'Maap CI',
    domain: 'https://apps.apple.com/us/app/xpenshare/id6741827585',
  },
});

const startAt = Date.now() - 60000;
const historicalAt = startAt - 10 * 60000;
const sdkDate = ms => new Date(ms).toISOString().replace(/\.\d{3}Z$/, '+0000');
const base = {
  appID,
  clientUser: 'b'.repeat(64),
  sessionID: randomUUID(),
  receivedAt: sdkDate(Date.now()),
  isTestMode: 'false',
  payload: { 'TelemetryDeck.AppInfo.version': '1.2.3' },
};
const screens = ['Home', 'Catalog', 'Detail'];
const destinations = ['/Home', '/Catalog?gclid=maap-ci', '/Detail'];
const navigation = await request('/v2/', {
  headers: {
    'cf-connecting-ip': '8.8.8.8',
    'cf-ipcountry': 'US',
    'cf-region-code': 'WA',
    'cf-ipcity': 'Seattle',
  },
  body: destinations.map(destination => ({
    ...base,
    type: 'TelemetryDeck.Navigation.pathChanged',
    payload: { ...base.payload, 'TelemetryDeck.Navigation.destinationPath': destination },
  })),
});
assert.equal(navigation.headers.get('x-maap-server'), 'rust');
assert.deepEqual(await navigation.json(), { accepted: 3, rejected: 0, dropped: 0 });
const actions = await request('/v2/namespace/ios/', {
  body: [
    { ...base, type: 'DemoEvent' },
    { ...base, type: 'OtherEvent' },
    { ...base, type: 'DemoEvent' },
    { ...base, type: 'TestOnly', isTestMode: 'true', receivedAt: sdkDate(historicalAt) },
  ],
});
assert.equal(actions.headers.get('x-maap-server'), 'rust');
assert.deepEqual(await actions.json(), { accepted: 4, rejected: 0, dropped: 0 });

const endAt = Date.now() + 1000;
async function metric(website, type, from = startAt, to = endAt, extra = {}) {
  const params = new URLSearchParams({ type, startAt: from, endAt: to, ...extra });
  const rows = await (await request(`/api/websites/${website}/metrics?${params}`, { token })).json();
  return rows.map(({ x }) => x).sort();
}
assert.deepEqual(await metric(appID, 'path'), screens.map(screen => `/${screen}`).sort());
assert.deepEqual(await metric(appID, 'fullPath'), destinations.sort());
assert.deepEqual(await metric(appID, 'path', startAt, endAt, { path: '/Catalog' }), ['/Catalog']);
assert.deepEqual(await metric(appID, 'entry'), ['/Home']);
assert.deepEqual(await metric(appID, 'exit'), ['/Detail']);
assert.deepEqual(await metric(appID, 'channel'), ['paidAds']);
assert.deepEqual(await metric(appID, 'referrer'), []);
assert.deepEqual(await metric(appID, 'browser'), ['App 1.2.3']);
assert.deepEqual(await metric(appID, 'country'), ['US']);
assert.deepEqual(await metric(appID, 'region'), ['US-WA']);
assert.deepEqual(await metric(appID, 'city'), ['Seattle']);
assert.deepEqual(await metric(appID, 'event', historicalAt - 1000, endAt), ['DemoEvent', 'OtherEvent']);
assert.deepEqual(await metric(appID, 'event', historicalAt - 1000, endAt, { limit: 1 }), ['DemoEvent']);
assert.deepEqual(
  await metric(appID, 'event', historicalAt - 1000, endAt, { limit: 1, offset: 1 }),
  ['OtherEvent'],
);
assert.deepEqual(await metric(appID, 'event', historicalAt - 1000, historicalAt + 1000, { tag: 'test' }), ['TestOnly']);
const filter = new URLSearchParams({ startAt, endAt, browser: 'App 1.2.3' });
const filtered = await request(`/api/websites/${appID}/stats?${filter}`, { token });
const filteredStats = await filtered.json();
assert.equal(Number(filteredStats.pageviews), 3);
assert.equal(Number(filteredStats.comparison.pageviews), 0);
const invalidMetric = await fetch(
  `${origin}/api/websites/${appID}/metrics?${new URLSearchParams({
    type: 'not-a-dimension',
    startAt,
    endAt,
  })}`,
  { headers: { authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(30000) },
);
assert.equal(new URL(invalidMetric.url).origin, origin);
assert.equal(invalidMetric.status, 400);
assert.equal(invalidMetric.headers.get('x-maap-server'), 'rust');

const { key } = await (await request('/api/me/api-keys', { token, body: { name: 'CI' } })).json();
assert.ok(key);
const preflight = await fetch(`${origin}/api/send`, {
  method: 'OPTIONS',
  headers: { origin: 'https://tracker.example', 'access-control-request-method': 'POST',
    'access-control-request-headers': 'content-type,x-umami-website-id' },
});
assert.ok(preflight.ok);
assert.equal(preflight.headers.get('access-control-allow-origin'), '*');
assert.match(preflight.headers.get('access-control-allow-methods') || '', /POST/);
// The stateless MCP transport can reply with JSON or an SSE message.
const version = '2026-07-28';
const rpc = await request('/mcp', {
  token: key,
  headers: {
    accept: 'application/json, text/event-stream',
    'mcp-protocol-version': version,
    'mcp-method': 'tools/call',
    'mcp-name': 'get_website_stats',
  },
  body: {
    id: 1, method: 'tools/call', jsonrpc: '2.0',
    params: {
      name: 'get_website_stats',
      arguments: {
        websiteId: appID,
        startAt: new Date(startAt).toISOString(),
        endAt: new Date(endAt).toISOString(),
      },
      _meta: {
        'io.modelcontextprotocol/protocolVersion': version,
        'io.modelcontextprotocol/clientInfo': { name: 'maap-ci', version: '1' },
        'io.modelcontextprotocol/clientCapabilities': {},
      },
    },
  },
});
const reply = rpc.headers.get('content-type')?.includes('text/event-stream')
  ? (await rpc.text()).split('\n').filter(line => line.startsWith('data:'))
      .map(line => JSON.parse(line.slice(5))).find(message => message.id === 1)
  : await rpc.json();
assert.equal(reply?.result?.structuredContent?.current?.pageviews, 3);

// Verify static app routes (/login, /websites), /_next/static chunks, and /app-icon -> /images/os/ios.png
// are served directly by the Rust server (including when Node is completely severed).
for (const appRoute of ['/login', '/websites', `/websites/${appID}`]) {
  const page = await request(appRoute);
  assert.equal(page.headers.get('x-maap-server'), 'rust', `${appRoute} served by Rust`);
  assert.match(
    page.headers.get('content-security-policy') || '',
    /frame-ancestors 'self'/,
    `${appRoute} sets CSP frame-ancestors`,
  );
  assert.equal(page.headers.get('x-content-type-options'), 'nosniff');
  const html = await page.text();
  assert.match(html, /Maap/);
  const chunkMatch = html.match(/\/_next\/static\/[^"'\s>]+\.js/);
  assert.ok(chunkMatch, `${appRoute} references a /_next/static JS bundle`);
  const chunk = await request(chunkMatch[0]);
  assert.equal(chunk.headers.get('x-maap-server'), 'rust');
  assert.match(chunk.headers.get('cache-control') || '', /immutable/);
}
for (const apiPath of [
  '/api/auth/verify',
  '/api/2fa/status',
  '/api/me/websites',
  `/api/users/${user.id}/websites`,
  `/api/websites/${appID}`,
  `/api/websites/charts?ids=${appID}&startAt=${startAt}&endAt=${endAt}`,
  `/api/websites/${appID}/pageviews?startAt=${startAt}&endAt=${endAt}`,
  `/api/websites/${appID}/active`,
  `/api/realtime/${appID}`,
  `/api/websites/${appID}/events?startAt=${startAt}&endAt=${endAt}`,
  `/api/websites/${appID}/sessions?startAt=${startAt}&endAt=${endAt}`,
  `/api/websites/${appID}/sessions/stats?startAt=${startAt}&endAt=${endAt}`,
  `/api/websites/${appID}/daterange`,
]) {
  const res = await request(apiPath, { token });
  assert.equal(res.headers.get('x-maap-server'), 'rust', `${apiPath} served by Rust`);
}
const fallbackIcon = await request('/app-icon?url=https://apps.apple.com/app/id999999999');
assert.equal(fallbackIcon.headers.get('x-maap-server'), 'rust');
assert.equal(fallbackIcon.headers.get('content-type'), 'image/png');
const brandLogo = await request('/logo.png');
assert.equal(brandLogo.headers.get('x-maap-server'), 'rust');
assert.equal(brandLogo.headers.get('content-type'), 'image/png');
console.log('Verified both ingestion paths, pageviews, Browser, test isolation, static app routes and MCP stats.');
