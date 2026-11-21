import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  forbiddenResponse,
  jsonResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { appSettingsInputSchema, appSettingsResponseSchema } from './schema';

const websitePathSchema = z.object({
  websiteId: z.string(),
});

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-settings',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'getWebsiteAppSettings',
      summary: 'Get website app settings',
      description:
        'Returns persisted app presentation settings, defaults for unset values, and write capability.',
      tags: ['Websites'],
      requestParams: { path: websitePathSchema },
      responses: {
        '200': jsonResponse(appSettingsResponseSchema, 'Website app settings.'),
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
      },
    },
  }),
  defineOperation({
    method: 'post',
    path: '/api/websites/{websiteId}/app-settings',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'updateWebsiteAppSettings',
      summary: 'Update website app settings',
      description: 'Replaces the complete persisted app settings record for the website.',
      tags: ['Websites'],
      requestParams: { path: websitePathSchema },
      requestBody: {
        required: true,
        content: {
          'application/json': {
            schema: appSettingsInputSchema,
          },
        },
      },
      responses: {
        '200': jsonResponse(appSettingsResponseSchema, 'Website app settings updated.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
      },
    },
  }),
] as const;
