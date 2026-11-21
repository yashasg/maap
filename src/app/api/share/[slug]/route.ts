import { resolveBoardShareScope } from '@/lib/board-share';
import { ENTITY_TYPE, ROLES, SHARE_TOKEN_TYPE } from '@/lib/constants';
import { secret } from '@/lib/crypto';
import { createToken } from '@/lib/jwt';
import prisma from '@/lib/prisma';
import redis from '@/lib/redis';
import { json, notFound } from '@/lib/response';
import type { WhiteLabel } from '@/lib/types';
import { getLink, getPixel, getShareByCode, getWebsite } from '@/queries/prisma';

async function getAccountId(entity: { userId?: string; teamId?: string }): Promise<string | null> {
  if (entity.userId) {
    return entity.userId;
  }

  if (entity.teamId) {
    const teamOwner = await prisma.client.teamUser.findFirst({
      where: {
        teamId: entity.teamId,
        role: ROLES.teamOwner,
      },
      select: {
        userId: true,
      },
    });

    return teamOwner?.userId || null;
  }

  return null;
}

async function getWhiteLabel(accountId: string): Promise<WhiteLabel | null> {
  if (!redis.enabled) {
    return null;
  }

  const data = await redis.client.get(`white-label:${accountId}`);

  if (data) {
    return data as WhiteLabel;
  }

  return null;
}

export async function GET(_request: Request, { params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;

  const share = await getShareByCode(slug);

  if (!share) {
    return notFound();
  }

  const data: Record<string, any> = {
    shareId: share.id,
    shareType: share.shareType,
    parameters: share.parameters,
  };

  let entity: { userId?: string; teamId?: string } | null = null;

  if (share.shareType === ENTITY_TYPE.board) {
    const resolved = await resolveBoardShareScope(share.entityId);
    if (!resolved) return notFound();
    entity = resolved.board;
    data.boardId = share.entityId;
    data.websiteIds = resolved.scope.websiteIds;
    data.pixelIds = resolved.scope.pixelIds;
    data.linkIds = resolved.scope.linkIds;
  } else if (share.shareType === ENTITY_TYPE.website) {
    entity = await getWebsite(share.entityId);
    if (!entity) return notFound();
    data.websiteId = share.entityId;
  } else if (share.shareType === ENTITY_TYPE.pixel) {
    entity = await getPixel(share.entityId);
    if (!entity) return notFound();
    data.websiteId = share.entityId;
    data.pixelId = share.entityId;
  } else if (share.shareType === ENTITY_TYPE.link) {
    entity = await getLink(share.entityId);
    if (!entity) return notFound();
    data.websiteId = share.entityId;
    data.linkId = share.entityId;
  } else {
    return notFound();
  }

  data.token = createToken({ ...data, type: SHARE_TOKEN_TYPE }, secret());

  const accountId = await getAccountId(entity);

  if (accountId) {
    const whiteLabel = await getWhiteLabel(accountId);
    if (whiteLabel) {
      data.whiteLabel = whiteLabel;
    }
  }

  return json(data);
}
