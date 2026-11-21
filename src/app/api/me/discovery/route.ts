import prisma from '@/lib/prisma';
import { parseRequest } from '@/lib/request';
import { json, serverError } from '@/lib/response';
import { discoveryChoiceSchema, discoveryInputSchema } from './contract';

const SETTING_KEY_PREFIX = 'workspaceDiscovery:';

function getSettingKey(userId: string) {
  return `${SETTING_KEY_PREFIX}${userId}`;
}

function privateJson(data: Record<string, unknown>) {
  const response = json(data);
  response.headers.set('Cache-Control', 'private, no-store, max-age=0');
  return response;
}

export async function GET(request: Request) {
  const { auth, error } = await parseRequest(request);

  if (error) {
    return error();
  }

  const setting = await prisma.client.appSetting.findUnique({
    where: { key: getSettingKey(auth.user.id) },
  });

  if (!setting) {
    return privateJson({ choice: null });
  }

  const choice = discoveryChoiceSchema.safeParse(setting.value);

  if (!choice.success) {
    return serverError('Invalid saved discovery choice');
  }

  return privateJson({ choice: choice.data });
}

export async function POST(request: Request) {
  const { auth, body, error } = await parseRequest(request, discoveryInputSchema);

  if (error) {
    return error();
  }

  const setting = await prisma.client.appSetting.upsert({
    where: { key: getSettingKey(auth.user.id) },
    update: { value: body.choice },
    create: {
      key: getSettingKey(auth.user.id),
      value: body.choice,
    },
  });

  return privateJson({ choice: setting.value });
}
