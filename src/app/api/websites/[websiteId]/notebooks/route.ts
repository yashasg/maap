import { uuid } from '@/lib/crypto';
import prisma, { getRawQueryClient } from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { json, unauthorized } from '@/lib/response';
import { canUpdateWebsite, canViewWebsite } from '@/permissions';
import { notebookInputSchema } from './schema';

const fields = `
  notebook_id AS "id",
  website_id AS "websiteId",
  created_by AS "createdBy",
  title,
  created_at AS "createdAt",
  updated_at AS "updatedAt"
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

  const data = await prisma.client.$queryRawUnsafe(
    `SELECT ${fields}
     FROM notebook
     WHERE website_id = $1
       AND EXISTS (
         SELECT 1
         FROM website
         WHERE website_id = $1 AND deleted_at IS NULL
       )
     ORDER BY updated_at DESC, created_at DESC`,
    websiteId,
  );
  const canWrite = await canUpdateWebsite(auth, websiteId);

  return json({ data, canWrite });
}

export async function POST(
  request: Request,
  { params }: { params: Promise<{ websiteId: string }> },
) {
  const { auth, body, error } = await parseRequest(request, notebookInputSchema);
  if (error) return error();

  const { websiteId } = await params;
  if (!auth.user || !(await canUpdateWebsite(auth, websiteId))) {
    return unauthorized();
  }

  const client = getRawQueryClient(prisma.client as any, { write: true });
  const [result] = (await client.$queryRawUnsafe(
    `WITH live_website AS (
       SELECT website_id
       FROM website
       WHERE website_id = $2 AND deleted_at IS NULL
       FOR UPDATE
     )
     INSERT INTO notebook (notebook_id, website_id, created_by, title, content, created_at, updated_at)
     SELECT $1, website_id, $3, $4, $5::jsonb, now(), now()
     FROM live_website
     RETURNING ${fields}, content`,
    uuid(),
    websiteId,
    auth.user.id,
    body.title,
    JSON.stringify(body.content),
  )) as any[];
  if (!result) {
    return unauthorized();
  }

  return json(result);
}
