import prisma, { getRawQueryClient } from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { json, notFound, unauthorized } from '@/lib/response';
import { canUpdateWebsite, canViewWebsite } from '@/permissions';
import { notebookInputSchema } from '../schema';

const fields = `
  notebook_id AS "id",
  website_id AS "websiteId",
  created_by AS "createdBy",
  title,
  content,
  created_at AS "createdAt",
  updated_at AS "updatedAt"
`;

export async function GET(
  request: Request,
  { params }: { params: Promise<{ websiteId: string; notebookId: string }> },
) {
  const { auth, error } = await parseRequest(request);
  if (error) return error();

  const { websiteId, notebookId } = await params;
  if (!auth.user || !(await canViewWebsite({ user: auth.user }, websiteId))) {
    return unauthorized();
  }

  const [data] = (await prisma.client.$queryRawUnsafe(
    `SELECT ${fields}
     FROM notebook
     WHERE website_id = $1
       AND notebook_id = $2
       AND EXISTS (
         SELECT 1
         FROM website
         WHERE website_id = $1 AND deleted_at IS NULL
       )`,
    websiteId,
    notebookId,
  )) as any[];
  if (!data) return notFound();

  return json({ data, canWrite: await canUpdateWebsite(auth, websiteId) });
}

export async function POST(
  request: Request,
  { params }: { params: Promise<{ websiteId: string; notebookId: string }> },
) {
  const { auth, body, error } = await parseRequest(request, notebookInputSchema);
  if (error) return error();

  const { websiteId, notebookId } = await params;
  if (!auth.user || !(await canUpdateWebsite(auth, websiteId))) {
    return unauthorized();
  }

  const client = getRawQueryClient(prisma.client as any, { write: true });
  const [result] = (await client.$queryRawUnsafe(
    `WITH live_website AS (
       SELECT website_id
       FROM website
       WHERE website_id = $1 AND deleted_at IS NULL
       FOR UPDATE
     )
     UPDATE notebook
     SET title = $3, content = $4::jsonb, updated_at = now()
     WHERE website_id = $1
       AND notebook_id = $2
       AND EXISTS (SELECT 1 FROM live_website)
     RETURNING ${fields}`,
    websiteId,
    notebookId,
    body.title,
    JSON.stringify(body.content),
  )) as any[];
  if (!result) return notFound();

  return json(result);
}
