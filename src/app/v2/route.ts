import { POST as send } from '@/app/api/send/route';
import { corsPreflight, withCorsHeaders } from '@/lib/cors';
import { ingestBatch } from '@/lib/telemetrydeck';

export function OPTIONS() {
  return corsPreflight();
}

export async function POST(request: Request) {
  const response = await ingestBatch(request, send, {
    hostname: process.env.TD_HOSTNAME,
    testWebsiteId: process.env.TD_TEST_WEBSITE_ID,
    allowedKeys: new Set(
      process.env.TD_ALLOWED_KEYS?.split(',')
        .map(key => key.trim())
        .filter(Boolean),
    ),
  });
  return withCorsHeaders(response);
}
