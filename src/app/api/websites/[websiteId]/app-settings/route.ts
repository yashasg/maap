import { type AppSettingsResponse, DEFAULT_APP_SETTINGS } from '@/lib/app-settings';
import prisma, { getRawQueryClient } from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { json, unauthorized } from '@/lib/response';
import { canUpdateWebsite, canViewWebsite } from '@/permissions';
import { appSettingsInputSchema } from './schema';

const fields = `
  website_id AS "websiteId",
  overview_layout AS "overviewLayout",
  color_scheme AS "colorScheme",
  revenue_display_mode AS "revenueDisplayMode"
`;

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, error } = await parseRequest(request);
  if (error) return error();

  const { websiteId } = await params;
  if (!auth.user || !(await canViewWebsite({ user: auth.user }, websiteId))) {
    return unauthorized();
  }

  const rows = (await prisma.client.$queryRawUnsafe(
    `SELECT ${fields}
     FROM app_settings
     WHERE website_id = $1
       AND EXISTS (
         SELECT 1
         FROM website
         WHERE website_id = $1 AND deleted_at IS NULL
       )`,
    websiteId,
  )) as AppSettingsResponse[];
  const canWrite = await canUpdateWebsite(auth, websiteId);

  return json({
    websiteId,
    ...DEFAULT_APP_SETTINGS,
    ...rows[0],
    canWrite,
  });
}

export async function POST(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, body, error } = await parseRequest(request, appSettingsInputSchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!auth.user || !(await canUpdateWebsite(auth, websiteId))) {
    return unauthorized();
  }

  const client = getRawQueryClient(prisma.client as any, { write: true });
  const [settings] = (await client.$queryRawUnsafe(
    `WITH live_website AS (
       SELECT website_id
       FROM website
       WHERE website_id = $1 AND deleted_at IS NULL
       FOR UPDATE
     )
     INSERT INTO app_settings
       (website_id, overview_layout, color_scheme, revenue_display_mode, created_at, updated_at)
     SELECT website_id, $2, $3, $4, now(), now()
     FROM live_website
     ON CONFLICT (website_id) DO UPDATE SET
       overview_layout = EXCLUDED.overview_layout,
       color_scheme = EXCLUDED.color_scheme,
       revenue_display_mode = EXCLUDED.revenue_display_mode,
       updated_at = now()
     RETURNING ${fields}`,
    websiteId,
    body.overviewLayout,
    body.colorScheme,
    body.revenueDisplayMode,
  )) as AppSettingsResponse[];
  if (!settings) {
    return unauthorized();
  }

  return json({ ...settings, canWrite: true });
}
