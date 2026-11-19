import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmod, copyFile, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { GET as getAppIcon, resolveAppIconUrl } from '../../src/app/app-icon/route.ts';
import { DOMAIN_REGEX } from '../../src/lib/constants.ts';
import { ingestBatch, mapSignal } from '../../src/lib/telemetrydeck.ts';

const [navigation, action] = JSON.parse(
  await readFile(new URL('./representative-swift-batch.json', import.meta.url), 'utf8'),
);
const now = Date.parse('2026-03-01T10:23:00Z');

test('maps navigation to a pageview with app dimensions and only selected SDK properties', () => {
  const { type, payload } = mapSignal(navigation, {}, now);
  assert.equal(type, 'event');
  assert.equal(payload.name, undefined);
  assert.equal(payload.url, '/library');
  assert.equal(payload.website, navigation.appID.toLowerCase());
  assert.equal(payload.hostname, 'app.ios');
  assert.equal(payload.id, navigation.clientUser);
  assert.ok(!JSON.stringify(payload).includes(navigation.sessionID));
  assert.equal(payload.timestamp, undefined);
  assert.equal(payload.device, 'tablet');
  assert.equal(payload.os, 'iOS 18.4');
  assert.equal(payload.screen, '834x1194');
  assert.equal(payload.language, 'fr-CA');
  assert.equal(payload.browser, 'App 1.2.3');
  assert.match(payload.userAgent, /iPad; CPU OS 18_4_1.*Version\/18\.4.*Safari/);
  assert.deepEqual({ ...payload.data }, {
    shelf: 'fiction',
    'TelemetryDeck.AppInfo.version': '1.2.3',
    'TelemetryDeck.AppInfo.buildNumber': '42',
    'TelemetryDeck.Device.modelName': 'iPad14,3',
    'TelemetryDeck.Device.systemVersion': 'iOS 18.4.1',
    appVersion: '1.2.3',
    buildNumber: '42',
    modelName: 'iPad14,3',
    systemVersion: 'iOS 18.4.1',
    locale: 'fr_CA',
    region: 'CA',
  });
});

test('keeps custom signals named, promotes floatValue, supports aliases and exact opt-ins', () => {
  const { payload } = mapSignal(
    {
      ...action,
      floatValue: 0,
      payload: {
        ...action.payload,
        appVersion: 'wrong', // Canonical key wins over its deprecated alias.
        modelName: 'iPhone17,1',
        operatingSystem: 'iOS',
        majorMinorSystemVersion: '18.4',
        preferredLanguage: 'en-US',
        'TelemetryDeck.RunContext.isTestFlight': 'true',
        'TelemetryDeck.RunContext.locale': 'en_US',
        isTestFlight: 'true',
        value: '003', // Never coerce app-defined strings to numbers.
      },
    },
    {
      hostname: 'library.ios',
      allowedKeys: new Set(['TelemetryDeck.RunContext.isTestFlight', 'value']),
    },
    now,
  );
  assert.equal(payload.name, action.type);
  assert.equal(payload.url, '/');
  assert.equal(payload.hostname, 'library.ios');
  assert.equal(payload.browser, 'App 1.2.3');
  assert.equal(payload.os, 'iOS 18.4');
  assert.equal(payload.language, 'en-US');
  assert.equal(payload.device, 'mobile');
  assert.match(payload.userAgent, /iPhone; CPU iPhone OS 18_4.*Safari/);
  assert.deepEqual({ ...payload.data }, {
    value: '003',
    'TelemetryDeck.RunContext.isTestFlight': 'true',
    floatValue: 0,
  });
});

test('only signals older than five minutes enter send historical mode', () => {
  const at = Date.parse(action.receivedAt);
  const mapAt = now => mapSignal(action, {}, now).payload.timestamp;
  assert.equal(mapAt(at - 600000), undefined); // Device clock ahead.
  assert.equal(mapAt(at + 300000), undefined);
  assert.equal(mapAt(at + 300001), at / 1000);
  assert.equal(
    mapSignal({ ...action, receivedAt: '2026-03-01T05:20:31.999-05:00' }, {}, at + 301000)
      .payload.timestamp,
    at / 1000,
  );
});

test('test-mode signals tag events with test on the app website, or use testWebsiteId override', () => {
  const options = { testWebsiteId: '51e5d7e8-47d1-48fe-8545-59be78020636' };
  for (const isTestMode of ['true', true]) {
    const signal = { ...action, isTestMode };
    const mapped = mapSignal(signal);
    assert.equal(mapped.payload.website, action.appID.toLowerCase());
    assert.equal(mapped.payload.tag, 'test');
    assert.equal(mapSignal(signal, options).payload.website, options.testWebsiteId);
    assert.equal(mapSignal(signal, options).payload.tag, 'test');
  }
  assert.equal(mapSignal(action, options).payload.website, action.appID.toLowerCase());
  assert.equal(mapSignal(action, options).payload.tag, undefined);
  assert.throws(() => mapSignal({ ...action, isTestMode: 'unknown' }));
});

test('maps macOS and Android signals to platform hostnames, devices, and user agents', () => {
  const mac = mapSignal({
    ...action,
    payload: {
      ...action.payload,
      'TelemetryDeck.Device.operatingSystem': 'macOS',
      'TelemetryDeck.Device.systemMajorMinorVersion': 'macOS 15.2',
      'TelemetryDeck.Device.systemVersion': 'macOS 15.2.1',
      'TelemetryDeck.Device.modelName': 'Mac16,1',
    },
  }).payload;
  assert.equal(mac.hostname, 'app.macos');
  assert.equal(mac.device, 'desktop');
  assert.equal(mac.os, 'macOS 15.2');
  assert.match(mac.userAgent, /Macintosh; Intel Mac OS X 15_2_1.*Safari/);

  const android = mapSignal({
    ...action,
    payload: {
      ...action.payload,
      'TelemetryDeck.Device.operatingSystem': 'Android',
      'TelemetryDeck.Device.systemMajorMinorVersion': 'Android 15',
      'TelemetryDeck.Device.systemVersion': '15',
      'TelemetryDeck.Device.modelName': 'Pixel 9',
    },
  }).payload;
  assert.equal(android.hostname, 'app.android');
  assert.equal(android.device, 'mobile');
  assert.equal(android.os, 'Android 15');
  assert.match(android.userAgent, /Linux; Android 15; Pixel 9.*Mobile Safari/);
});

function request(body, headers = {}) {
  return new Request('https://metrics.example/v2/namespace/ios/', {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...headers },
    body,
  });
}

test('isolates partial failures and forwards fresh POSTs with client context', async t => {
  const log = t.mock.method(console, 'warn', () => {});
  const req = request(
    JSON.stringify([
      null,
      navigation,
      { ...action, clientUser: 'private-raw-id' },
      { ...action, receivedAt: 'invalid' },
      { ...action, payload: ['bad'] },
      { ...action, isTestMode: 'true' },
      { ...action, type: 'UnknownWebsite' },
      { ...action, type: 'WriterUnavailable' },
      action,
    ]),
    {
      'x-forwarded-for': '203.0.113.1',
      'cf-ipcountry': 'CA',
      'user-agent': 'App/42 CFNetwork',
      'content-length': '12345',
    },
  );
  const calls = [];
  const response = await ingestBatch(req, async r => {
    const { payload } = await r.json();
    calls.push({ request: r, payload });
    if (payload.name === 'UnknownWebsite') return Response.json({}, { status: 400 });
    if (payload.name === 'WriterUnavailable') throw new Error('private database details');
    return Response.json({ cache: 'private-token', sessionId: 'private-session' });
  });

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { accepted: 3, rejected: 6, dropped: 0 });
  assert.deepEqual(calls.map(c => ({ name: c.payload.name, tag: c.payload.tag })), [
    { name: undefined, tag: undefined },
    { name: action.type, tag: 'test' },
    { name: 'UnknownWebsite', tag: undefined },
    { name: 'WriterUnavailable', tag: undefined },
    { name: action.type, tag: undefined },
  ]);
  assert.equal(new Set(calls.map(c => c.request)).size, 5);
  for (const { request: r } of calls) {
    assert.equal(r.method, 'POST');
    assert.equal(r.url, 'https://metrics.example/api/send');
    assert.equal(r.headers.get('x-forwarded-for'), '203.0.113.1');
    assert.equal(r.headers.get('cf-ipcountry'), 'CA');
    assert.equal(r.headers.get('user-agent'), 'App/42 CFNetwork');
    assert.equal(r.headers.get('content-type'), 'application/json');
    assert.equal(r.headers.get('content-length'), null);
  }
  assert.equal(req.headers.get('content-length'), '12345');
  assert.equal(log.mock.callCount(), 1);
  assert.doesNotMatch(JSON.stringify(log.mock.calls[0].arguments), /private|library|203\.0\.113/);
});

test('requires a JSON array, but acknowledges empty and wholly rejected batches', async t => {
  const send = t.mock.fn(async () => Response.json({}, { status: 503 }));
  t.mock.method(console, 'warn', () => {});
  for (const malformed of ['', '{', 'null', '{}', JSON.stringify(action)]) {
    assert.equal((await ingestBatch(request(malformed), send)).status, 400);
  }
  const empty = await ingestBatch(request('[]'), send);
  assert.deepEqual(await empty.json(), { accepted: 0, rejected: 0, dropped: 0 });
  assert.equal(send.mock.callCount(), 0);
  const failed = await ingestBatch(request(JSON.stringify([action])), send);
  assert.equal(failed.status, 200);
  assert.deepEqual(await failed.json(), { accepted: 0, rejected: 1, dropped: 0 });
});

test('deploy.sh validates 40-hex SHA tags and updates .env atomically while preserving secrets', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'maap-deploy-test-'));
  try {
    const script = join(dir, 'deploy.sh');
    await copyFile(new URL('../../deploy/maap/deploy.sh', import.meta.url), script);
    await chmod(script, 0o755);

    const bin = join(dir, 'bin');
    await writeFile(
      join(dir, 'docker'),
      '#!/bin/bash\nprintf "%s\\n" "$*" >> docker.log\nexit 0\n',
      { mode: 0o755 },
    );
    await writeFile(
      join(dir, '.env'),
      'POSTGRES_PASSWORD=keep-db\nAPP_SECRET=keep-secret\nTWO_FACTOR_ENCRYPTION_KEY=keep-2fa\nUMAMI_IMAGE_TAG=0000000000000000000000000000000000000000\nTUNNEL_TOKEN=pending-cloudflare-tunnel-token\n',
      { mode: 0o600 },
    );

    for (const invalid of ['', 'latest', 'abc123', 'f335d2b06f275c3c6fa9552bfbb3cf546f4e95cd;rm']) {
      assert.throws(() =>
        execFileSync('bash', [script, invalid], {
          cwd: dir,
          env: { ...process.env, PATH: `${dir}:${process.env.PATH}` },
          stdio: 'pipe',
        }),
      );
    }

    const sha = 'f335d2b06f275c3c6fa9552bfbb3cf546f4e95cd';
    const out = execFileSync('bash', [script, sha], {
      cwd: dir,
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, TUNNEL_TOKEN: 'eyJ-live-tunnel' },
      encoding: 'utf8',
    });
    assert.match(out, new RegExp(`Deployed ${sha}`));
    const envText = await readFile(join(dir, '.env'), 'utf8');
    assert.match(envText, new RegExp(`^UMAMI_IMAGE_TAG=${sha}$`, 'm'));
    assert.match(envText, /^TUNNEL_TOKEN=eyJ-live-tunnel$/m);
    assert.match(envText, /^POSTGRES_PASSWORD=keep-db$/m);
    assert.match(envText, /^APP_SECRET=keep-secret$/m);
    assert.match(envText, /^TWO_FACTOR_ENCRYPTION_KEY=keep-2fa$/m);
    assert.equal((await stat(join(dir, '.env'))).mode & 0o777, 0o600);
    const dockerLog = await readFile(join(dir, 'docker.log'), 'utf8');
    assert.match(dockerLog, /find \.next -type f/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test('accepts App Store and Google Play URLs in DOMAIN_REGEX and resolves store icons', async () => {
  for (const valid of [
    'example.com',
    'app.ios',
    'https://apps.apple.com/us/app/xpenshare/id6741827585',
    'https://apps.apple.com/app/id6741827585?mt=12',
    'https://play.google.com/store/apps/details?id=com.yashasg.receiptscanner&hl=en_US',
  ]) {
    assert.match(valid, DOMAIN_REGEX);
  }

  const appleIcon = await resolveAppIconUrl(
    'https://apps.apple.com/us/app/xpenshare/id6741827585',
    async url => {
      assert.match(String(url), /itunes\.apple\.com\/lookup\?id=6741827585&country=us/);
      return Response.json({
        results: [{ artworkUrl100: 'https://is1-ssl.mzstatic.com/image/thumb/icon100.jpg' }],
      });
    },
  );
  assert.equal(appleIcon, 'https://is1-ssl.mzstatic.com/image/thumb/icon100.jpg');

  const playIcon = await resolveAppIconUrl(
    'https://play.google.com/store/apps/details?id=com.yashasg.receiptscanner',
    async () =>
      new Response(
        '<html><head><meta property="og:image" content="https://play-lh.googleusercontent.com/app-icon"></head></html>',
      ),
  );
  assert.equal(playIcon, 'https://play-lh.googleusercontent.com/app-icon');
});

test('GET /app-icon caches resolved store artwork, sets 24h/7d Cache-Control, and falls back to OS icons', async t => {
  assert.equal((await getAppIcon(new Request('https://metrics.example/app-icon'))).status, 400);

  let upstreamCalls = 0;
  t.mock.method(globalThis, 'fetch', async url => {
    upstreamCalls++;
    const href = String(url);
    if (href.includes('itunes.apple.com/lookup?id=6741827585')) {
      return Response.json({
        results: [{ artworkUrl100: 'https://is1-ssl.mzstatic.com/image/thumb/xpenshare.png' }],
      });
    }
    if (href === 'https://is1-ssl.mzstatic.com/image/thumb/xpenshare.png') {
      return new Response(new Uint8Array([137, 80, 78, 71]), {
        headers: { 'content-type': 'image/png' },
      });
    }
    return new Response(null, { status: 404 });
  });

  const storeUrl = 'https://apps.apple.com/us/app/xpenshare/id6741827585';
  const reqUrl = `https://metrics.example/app-icon?url=${encodeURIComponent(storeUrl)}`;
  const first = await getAppIcon(new Request(reqUrl));
  assert.equal(first.status, 200);
  assert.equal(first.headers.get('content-type'), 'image/png');
  assert.equal(first.headers.get('cache-control'), 'public, max-age=86400, s-maxage=604800');
  assert.deepEqual(new Uint8Array(await first.arrayBuffer()), new Uint8Array([137, 80, 78, 71]));
  assert.equal(upstreamCalls, 2);

  // Second request for the same store URL is served from in-memory cache with 0 upstream calls.
  const second = await getAppIcon(new Request(reqUrl));
  assert.equal(second.status, 200);
  assert.equal(upstreamCalls, 2);

  // Unlisted/offline store URLs redirect (302) to the matching platform icon.
  const iosFallback = await getAppIcon(
    new Request('https://metrics.example/app-icon?url=https://apps.apple.com/app/id999999999'),
  );
  assert.equal(iosFallback.status, 302);
  assert.equal(iosFallback.headers.get('location'), 'https://metrics.example/images/os/ios.png');

  const macFallback = await getAppIcon(
    new Request('https://metrics.example/app-icon?url=https://apps.apple.com/app/id999999999?mt=12'),
  );
  assert.equal(macFallback.status, 302);
  assert.equal(macFallback.headers.get('location'), 'https://metrics.example/images/os/mac-os.png');

  const androidFallback = await getAppIcon(
    new Request(
      'https://metrics.example/app-icon?url=https://play.google.com/store/apps/details?id=com.missing.app',
    ),
  );
  assert.equal(androidFallback.status, 302);
  assert.equal(
    androidFallback.headers.get('location'),
    'https://metrics.example/images/os/android-os.png',
  );
});

test('every frontend query hook route in src/components/hooks/queries is registered in Rust router', async () => {
  const rustDir = new URL('../../rust/src/', import.meta.url);
  const rustFiles = (await readdir(rustDir)).filter(f => f.endsWith('.rs'));
  const rustSource = (
    await Promise.all(rustFiles.map(f => readFile(new URL(f, rustDir), 'utf8')))
  ).join('\n');

  const registeredPatterns = new Set();
  for (const match of rustSource.matchAll(/["'](\/api\/[^"']+)["']/g)) {
    registeredPatterns.add(match[1].replace(/\{[^}]+\}/g, ':param'));
  }

  const analyticsSource = await readFile(
    new URL('../../rust/src/analytics_api.rs', import.meta.url),
    'utf8',
  );
  const emptyArrayBlock =
    analyticsSource.match(
      /for path in \[([^\]]+)\]\s*\{\s*router = router\.route\(path, get\(empty_array\)\)/,
    )?.[1] || '';
  const emptyArrayPatterns = new Set(
    [...emptyArrayBlock.matchAll(/["'](\/api\/[^"']+)["']/g)].map(m =>
      m[1].replace(/\{[^}]+\}/g, ':param'),
    ),
  );

  const hooksDir = new URL('../../src/components/hooks/queries/', import.meta.url);
  const hookFiles = (await readdir(hooksDir)).filter(f => f.endsWith('.ts') && !f.endsWith('.test.ts'));
  const missing = [];
  const wrongPagedShape = [];

  for (const file of hookFiles) {
    const content = await readFile(new URL(file, hooksDir), 'utf8');
    const isPaged = content.includes('usePagedQuery');
    for (const match of content.matchAll(/(?:[`'"])(\/(?:admin|auth|2fa|me|dashboard|websites|realtime|teams|users|links|pixels|boards|share)[^`'"]*?)(?:[`'"])/g)) {
      const raw = match[1];
      if (raw === '/websites/${websiteId}/${feature}') {
        for (const feat of ['funnels', 'goals']) {
          const norm = `/api/websites/:param/${feat}`;
          if (!registeredPatterns.has(norm)) missing.push(`${file}: ${norm}`);
          if (isPaged && emptyArrayPatterns.has(norm)) wrongPagedShape.push(`${file}: ${norm}`);
        }
        continue;
      }
      if (raw.includes('${feature}') || raw.includes('${path}') || raw.includes('${scope}')) continue;
      const normalized = `/api${raw}`.replace(/\$\{[^}]+\}/g, ':param').replace(/\/+$/, '');
      if (!registeredPatterns.has(normalized)) {
        missing.push(`${file}: ${raw} (expected ${normalized})`);
      }
      if (isPaged && emptyArrayPatterns.has(normalized)) {
        wrongPagedShape.push(`${file}: ${normalized} uses usePagedQuery but is mapped to empty_array`);
      }
    }
  }

  assert.deepEqual(missing, []);
  assert.deepEqual(wrongPagedShape, []);
});

