import { POST as send } from '@/app/api/send/route';
import { ingestBatch } from '@/lib/telemetrydeck';

export async function POST(request: Request) {
  return ingestBatch(request, send, {
    hostname: process.env.TD_HOSTNAME,
    testWebsiteId: process.env.TD_TEST_WEBSITE_ID,
    allowedKeys: new Set(
      process.env.TD_ALLOWED_KEYS?.split(',').map(key => key.trim()).filter(Boolean),
    ),
  });
}
