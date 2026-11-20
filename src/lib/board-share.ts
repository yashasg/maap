import type { Board, Prisma } from '@/generated/prisma/client';
import { getBoardEntityIds } from '@/lib/boards';
import { ROLES } from '@/lib/constants';
import { getBillingAccess, getEntityBillingScope } from '@/lib/load';
import prisma from '@/lib/prisma';
import type { BoardParameters } from '@/lib/types';
import { getBoard, getUser } from '@/queries/prisma';

export type BoardShareScope = ReturnType<typeof getBoardEntityIds>;

type BoardShareOwner =
  | { type: 'team'; teamId: string }
  | { type: 'user'; userId: string; isAdmin: boolean };

function preserveSelectedOrder(ids: string[], allowed: { id: string }[]) {
  const allowedIds = new Set(allowed.map(({ id }) => id));

  return ids.filter(id => allowedIds.has(id));
}

async function filterBillingAccess<T extends { userId: string | null; teamId: string | null }>(
  owner: BoardShareOwner,
  entities: T[],
) {
  if (owner.type === 'team' || owner.isAdmin) {
    return entities;
  }

  const allowed = await Promise.all(
    entities.map(
      async entity => !(await getBillingAccess(await getEntityBillingScope(entity))).isPastDue,
    ),
  );

  return entities.filter((_entity, index) => allowed[index]);
}

function getOwnershipWhere(owner: BoardShareOwner) {
  if (owner.type === 'team') {
    return { teamId: owner.teamId };
  }

  if (owner.isAdmin) {
    return {};
  }

  return {
    OR: [
      { userId: owner.userId },
      {
        team: {
          deletedAt: null,
          members: {
            some: {
              userId: owner.userId,
            },
          },
        },
      },
    ],
  };
}

async function getBoardShareOwner(board: Board): Promise<BoardShareOwner | null> {
  if (!!board.userId === !!board.teamId) {
    return null;
  }

  if (board.teamId) {
    const team = await prisma.client.team.findFirst({
      where: {
        id: board.teamId,
        deletedAt: null,
      },
      select: {
        id: true,
      },
    });

    return team ? { type: 'team', teamId: board.teamId } : null;
  }

  const { userId } = board;

  if (!userId) {
    return null;
  }

  const user = await getUser(userId);

  return user
    ? {
        type: 'user',
        userId: user.id,
        isAdmin: user.role === ROLES.admin,
      }
    : null;
}

async function filterBoardEntityIds(
  ids: BoardShareScope,
  owner: BoardShareOwner,
): Promise<BoardShareScope> {
  const ownershipWhere = getOwnershipWhere(owner);
  const [websites, pixels, links] = await Promise.all([
    prisma.client.website.findMany({
      where: {
        id: { in: ids.websiteIds },
        deletedAt: null,
        ...ownershipWhere,
      } as Prisma.WebsiteWhereInput,
      select: { id: true, userId: true, teamId: true },
    }),
    prisma.client.pixel.findMany({
      where: {
        id: { in: ids.pixelIds },
        deletedAt: null,
        ...ownershipWhere,
      } as Prisma.PixelWhereInput,
      select: { id: true, userId: true, teamId: true },
    }),
    prisma.client.link.findMany({
      where: {
        id: { in: ids.linkIds },
        deletedAt: null,
        ...ownershipWhere,
      } as Prisma.LinkWhereInput,
      select: { id: true, userId: true, teamId: true },
    }),
  ]);
  const [billableWebsites, billablePixels, billableLinks] = await Promise.all([
    filterBillingAccess(owner, websites),
    filterBillingAccess(owner, pixels),
    filterBillingAccess(owner, links),
  ]);

  return {
    websiteIds: preserveSelectedOrder(ids.websiteIds, billableWebsites),
    pixelIds: preserveSelectedOrder(ids.pixelIds, billablePixels),
    linkIds: preserveSelectedOrder(ids.linkIds, billableLinks),
  };
}

export async function resolveBoardShareScope(
  boardId: string,
): Promise<{ board: Board; scope: BoardShareScope } | null> {
  const board = await getBoard(boardId);

  if (!board) {
    return null;
  }

  const owner = await getBoardShareOwner(board);

  if (!owner) {
    return null;
  }

  const selectedIds = getBoardEntityIds({
    type: board.type,
    parameters: board.parameters as BoardParameters,
  });

  return {
    board,
    scope: await filterBoardEntityIds(selectedIds, owner),
  };
}
