// SwiftSDK sends a JSON array, with SDK metadata and app-defined parameters sharing
// each signal's flat payload dictionary. Never spread a signal into an /api/send body.
const defaultSdkKeys = new Set([
  'TelemetryDeck.AppInfo.version',
  'TelemetryDeck.AppInfo.buildNumber',
  'TelemetryDeck.Device.modelName',
  'TelemetryDeck.Device.systemVersion',
]);

interface Options {
  hostname?: string;
  testWebsiteId?: string;
  allowedKeys?: ReadonlySet<string>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function dimension(value?: string) {
  const n = Number(value);
  return Number.isFinite(n) && Math.round(n) > 0 ? Math.round(n) : undefined;
}

function eventTimestamp(receivedAt: unknown, now: number) {
  if (receivedAt === undefined) return undefined;
  // Swift's yyyy-MM-dd'T'HH:mm:ssZ uses +0000. Also accept ISO offsets and fractions.
  if (
    typeof receivedAt !== 'string' ||
    !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:?\d\d)$/.test(receivedAt)
  ) {
    throw new Error('Invalid receivedAt');
  }
  const time = Date.parse(receivedAt);
  if (!Number.isFinite(time)) throw new Error('Invalid receivedAt');
  // Supplying *any* timestamp puts send into historical session mode. Let it use
  // its normal visit clock for recent signals (including devices ahead of us).
  return now - time > 5 * 60 * 1000 ? Math.floor(time / 1000) : undefined;
}

export function mapSignal(signal: unknown, options: Options = {}, now = Date.now()) {
  if (
    !isRecord(signal) ||
    typeof signal.type !== 'string' ||
    !signal.type.trim() ||
    typeof signal.appID !== 'string' ||
    !signal.appID.trim()
  ) {
    throw new Error('Missing signal type or appID');
  }

  const test = signal.isTestMode;
  if (test !== undefined && !['true', 'false', true, false].some(v => v === test)) {
    throw new Error('Invalid isTestMode');
  }
  const isTest = test === 'true' || test === true;
  // ponytail: tag='test' on the same appID website lets Umami's native tag filter power the Test Mode toggle without schema changes
  const website = isTest && options.testWebsiteId ? options.testWebsiteId : signal.appID;

  const properties = signal.payload === undefined ? {} : signal.payload;
  if (!isRecord(properties) || Object.values(properties).some(v => typeof v !== 'string')) {
    throw new Error('Invalid signal payload');
  }
  const props = properties as Record<string, string>;
  const data: Record<string, string | number> = Object.create(null);
  for (const [key, value] of Object.entries(props)) {
    const allowed = options.allowedKeys?.size
      ? options.allowedKeys.has(key)
      : !key.startsWith('TelemetryDeck.') || defaultSdkKeys.has(key);
    if (allowed) data[key] = value;
  }
  if (signal.floatValue !== undefined) {
    if (typeof signal.floatValue !== 'number' || !Number.isFinite(signal.floatValue)) {
      throw new Error('Invalid floatValue');
    }
    data.floatValue = signal.floatValue;
  }

  // The SDK sends a salted SHA-256 digest, not a raw ID. send truncates id to
  // FIELD_LENGTH.distinctId and owns session/visit resolution; don't use sessionID.
  const id = signal.clientUser;
  if (id !== undefined && (typeof id !== 'string' || !/^[0-9a-f]{64}$/i.test(id))) {
    throw new Error('Invalid clientUser');
  }

  const model = props['TelemetryDeck.Device.modelName'] ?? props.modelName;
  const device = model?.startsWith('iPad') ? 'tablet' : 'mobile';
  const appVersion = props['TelemetryDeck.AppInfo.version'] ?? props.appVersion;
  const osName = props['TelemetryDeck.Device.operatingSystem'] ?? props.operatingSystem;
  const version =
    props['TelemetryDeck.Device.systemMajorMinorVersion'] ?? props.majorMinorSystemVersion;
  // Swift's systemMajorMinorVersion already includes the platform ("iOS 17.4").
  const majorMinor = version?.match(/\d+(?:\.\d+)?$/)?.[0];
  const os = [osName, majorMinor].filter(Boolean).join(' ');
  const systemVersion = props['TelemetryDeck.Device.systemVersion'] ?? props.systemVersion;
  const agentVersion = systemVersion?.match(/\d+(?:\.\d+){0,2}$/)?.[0] ?? majorMinor ?? '17.0';
  const agentDevice = device === 'tablet' ? 'iPad; CPU OS' : 'iPhone; CPU iPhone OS';
  // Use Safari's UA shape rather than the SDK's CFNetwork agent. send still owns
  // fingerprinting, bot filtering and the original forwarded-IP / geo headers.
  const userAgent =
    `Mozilla/5.0 (${agentDevice} ${agentVersion.replaceAll('.', '_')} like Mac OS X) ` +
    `AppleWebKit/605.1.15 (KHTML, like Gecko) Version/${majorMinor ?? agentVersion} ` +
    'Mobile/15E148 Safari/604.1';
  const width = dimension(props['TelemetryDeck.Device.screenResolutionWidth']);
  const height = dimension(props['TelemetryDeck.Device.screenResolutionHeight']);
  const nav = signal.type === 'TelemetryDeck.Navigation.pathChanged';
  const destination = nav ? props['TelemetryDeck.Navigation.destinationPath'] : '';
  return {
    type: 'event',
    payload: {
      website: website.toLowerCase(),
      hostname: options.hostname || 'app.ios',
      id,
      timestamp: eventTimestamp(signal.receivedAt, now),
      userAgent,
      browser: appVersion ? `App ${appVersion}` : undefined,
      os: os || undefined,
      device,
      screen: width && height ? `${width}x${height}` : undefined,
      language: props['TelemetryDeck.UserPreference.language'] ?? props.preferredLanguage,
      url: `/${(destination || '').replace(/^[\\/]+/, '')}`,
      name: nav ? undefined : signal.type,
      tag: isTest ? 'test' : undefined,
      data,
    },
  };
}

export async function ingestBatch(
  request: Request,
  send: (request: Request) => Promise<Response>,
  options: Options = {},
) {
  let batch: unknown;
  try {
    batch = await request.json();
  } catch {
    // Non-JSON and non-array bodies are both malformed SDK requests.
  }
  if (!Array.isArray(batch)) {
    return Response.json({ error: 'Expected a JSON array of signals.' }, { status: 400 });
  }

  const headers = new Headers(request.headers);
  // Forward the original network context, but not framing for the SDK batch body.
  for (const key of ['content-length', 'content-encoding', 'transfer-encoding']) {
    headers.delete(key);
  }
  headers.set('content-type', 'application/json');
  const url = new URL('/api/send', request.url);
  let accepted = 0;
  let rejected = 0;
  let dropped = 0;
  for (const signal of batch) {
    try {
      const body = mapSignal(signal, options);
      if (!body) {
        dropped++;
        continue;
      }
      const response = await send(
        new Request(url, { method: 'POST', headers, body: JSON.stringify(body) }),
      );
      if (response.ok) accepted++;
      else rejected++;
    } catch {
      rejected++;
    }
  }
  // The SDK retries/drops whole batches, not individual signals. Never replay
  // accepted siblings. No client IDs, event properties, or send tokens in logs.
  if (rejected) console.warn('TelemetryDeck rejected signals:', rejected, 'of', batch.length);
  return Response.json({ accepted, rejected, dropped });
}
