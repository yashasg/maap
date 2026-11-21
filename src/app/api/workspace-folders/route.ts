import { hasPermission } from '@/lib/auth';
import { PERMISSIONS } from '@/lib/constants';
import { uuid } from '@/lib/crypto';
import { getBillingAccess } from '@/lib/load';
import prisma from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { json, ok, unauthorized } from '@/lib/response';
import { hasTeamBillingAccess } from '@/permissions/team';
import { createFolder, folderScope, moveWebsite } from './contract';

type WorkspaceScope = { userId: string; teamId: null } | { userId: null; teamId: string };

const PERSONAL_FOLDERS_SQL = `
  SELECT folder_id AS "id", name
  FROM workspace_folder
  WHERE user_id = $1 AND team_id IS NULL
  ORDER BY name ASC, created_at ASC
`;
const TEAM_FOLDERS_SQL = `
  SELECT folder_id AS "id", name
  FROM workspace_folder
  WHERE team_id = $1 AND user_id IS NULL
  ORDER BY name ASC, created_at ASC
`;
const PERSONAL_ASSIGNMENTS_SQL = `
  SELECT a.website_id AS "websiteId", a.folder_id AS "folderId"
  FROM website_folder_assignment a
  JOIN workspace_folder f ON f.folder_id = a.folder_id
  JOIN website w ON w.website_id = a.website_id
  WHERE f.user_id = $1 AND f.team_id IS NULL
    AND w.user_id = $1 AND w.team_id IS NULL
    AND w.deleted_at IS NULL
`;
const TEAM_ASSIGNMENTS_SQL = `
  SELECT a.website_id AS "websiteId", a.folder_id AS "folderId"
  FROM website_folder_assignment a
  JOIN workspace_folder f ON f.folder_id = a.folder_id
  JOIN website w ON w.website_id = a.website_id
  WHERE f.team_id = $1 AND f.user_id IS NULL
    AND w.team_id = $1 AND w.user_id IS NULL
    AND w.deleted_at IS NULL
`;

async function canWriteWorkspace(user: { isAdmin?: boolean; role: string }) {
  return user.isAdmin || (await hasPermission(user.role, PERMISSIONS.websiteUpdate));
}

async function authorizeScope(
  user: { id: string; isAdmin?: boolean; role: string } | undefined,
  teamId: string | undefined,
  write: boolean,
): Promise<WorkspaceScope | null> {
  if (!user || (write && !(await canWriteWorkspace(user)))) {
    return null;
  }

  if (!teamId) {
    if (!user.isAdmin && (await getBillingAccess({ accountId: user.id }, user.id)).isPastDue) {
      return null;
    }
    return { userId: user.id, teamId: null };
  }

  const membership = await prisma.client.teamUser.findFirst({
    where: {
      teamId,
      userId: user.id,
      team: { deletedAt: null },
    },
    select: { role: true },
  });

  if (
    !membership ||
    (write && !(await hasPermission(membership.role, PERMISSIONS.websiteUpdate))) ||
    !(await hasTeamBillingAccess(teamId))
  ) {
    return null;
  }

  return { userId: null, teamId };
}

export async function GET(request: Request) {
  const { auth, query, error } = await parseRequest(request, folderScope);

  if (error) {
    return error();
  }

  const scope = await authorizeScope(auth.user, query.teamId, false);

  if (!scope) {
    return unauthorized();
  }

  const scopeId = scope.teamId ?? scope.userId;
  const foldersSql = scope.teamId ? TEAM_FOLDERS_SQL : PERSONAL_FOLDERS_SQL;
  const assignmentsSql = scope.teamId ? TEAM_ASSIGNMENTS_SQL : PERSONAL_ASSIGNMENTS_SQL;
  const [folders, assignments] = await Promise.all([
    prisma.client.$queryRawUnsafe<Array<{ id: string; name: string }>>(foldersSql, scopeId),
    prisma.client.$queryRawUnsafe<Array<{ websiteId: string; folderId: string }>>(
      assignmentsSql,
      scopeId,
    ),
  ]);

  return json({ folders, assignments });
}

export async function POST(request: Request) {
  const { auth, body, error } = await parseRequest(request, createFolder);

  if (error) {
    return error();
  }

  const scope = await authorizeScope(auth.user, body.teamId, true);

  if (!scope) {
    return unauthorized();
  }

  const id = uuid();
  const [folder] = await prisma.client.$queryRawUnsafe<Array<{ id: string; name: string }>>(
    scope.teamId
      ? `INSERT INTO workspace_folder
           (folder_id, name, user_id, team_id, created_at, updated_at)
         VALUES ($1, $2, NULL, $3, now(), now())
         RETURNING folder_id AS "id", name`
      : `INSERT INTO workspace_folder
           (folder_id, name, user_id, team_id, created_at, updated_at)
         VALUES ($1, $2, $3, NULL, now(), now())
         RETURNING folder_id AS "id", name`,
    id,
    body.name,
    scope.teamId ?? scope.userId,
  );

  return json(folder);
}

export async function PUT(request: Request) {
  const { auth, body, error } = await parseRequest(request, moveWebsite);

  if (error) {
    return error();
  }

  const scope = await authorizeScope(auth.user, body.teamId, true);

  if (!scope) {
    return unauthorized();
  }

  const moved = await prisma.transaction(async tx => {
    const scopeId = scope.teamId ?? scope.userId;
    const websiteRows = (await tx.$queryRawUnsafe(
      scope.teamId
        ? `SELECT EXISTS(
             SELECT 1 FROM website
             WHERE website_id = $1 AND team_id = $2
               AND user_id IS NULL AND deleted_at IS NULL
           ) AS allowed`
        : `SELECT EXISTS(
             SELECT 1 FROM website
             WHERE website_id = $1 AND user_id = $2
               AND team_id IS NULL AND deleted_at IS NULL
           ) AS allowed`,
      body.websiteId,
      scopeId,
    )) as Array<{ allowed: boolean }>;
    const [website] = websiteRows;

    if (!website?.allowed) {
      return false;
    }

    if (body.folderId === null) {
      await tx.$executeRawUnsafe(
        'DELETE FROM website_folder_assignment WHERE website_id = $1',
        body.websiteId,
      );
      return true;
    }

    const folderRows = (await tx.$queryRawUnsafe(
      scope.teamId
        ? `SELECT EXISTS(
             SELECT 1 FROM workspace_folder
             WHERE folder_id = $1 AND team_id = $2 AND user_id IS NULL
           ) AS allowed`
        : `SELECT EXISTS(
             SELECT 1 FROM workspace_folder
             WHERE folder_id = $1 AND user_id = $2 AND team_id IS NULL
           ) AS allowed`,
      body.folderId,
      scopeId,
    )) as Array<{ allowed: boolean }>;
    const [folder] = folderRows;

    if (!folder?.allowed) {
      return false;
    }

    await tx.$executeRawUnsafe(
      `INSERT INTO website_folder_assignment
         (website_id, folder_id, created_at, updated_at)
       VALUES ($1, $2, now(), now())
       ON CONFLICT (website_id) DO UPDATE SET
         folder_id = EXCLUDED.folder_id, updated_at = now()`,
      body.websiteId,
      body.folderId,
    );
    return true;
  });

  return moved ? ok() : unauthorized();
}
