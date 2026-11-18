import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmod, copyFile, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
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
      '#!/bin/bash\nexit 0\n',
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
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
