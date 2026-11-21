import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';

export const discoveryChoiceSchema = z.enum([
  'search',
  'recommendation',
  'community',
  'content',
  'comparison',
  'other',
]);
export const discoveryInputSchema = z.strictObject({ choice: discoveryChoiceSchema });

const scope = { path: '/api/me/discovery', audience: 'internal', auth: 'bearer' } as const;
export const operations = [
  defineOperation({
    ...scope,
    method: 'get',
    operation: {
      operationId: 'getMeDiscovery',
      summary: 'Get the current user’s discovery survey choice',
      tags: ['Me'],
      responses: {
        200: jsonResponse(z.object({ choice: discoveryChoiceSchema.nullable() })),
        401: unauthorizedResponse,
        500: serverErrorResponse,
      },
    },
  }),
  defineOperation({
    ...scope,
    method: 'post',
    operation: {
      operationId: 'postMeDiscovery',
      summary: 'Save the current user’s discovery survey choice',
      tags: ['Me'],
      requestBody: {
        required: true,
        content: { 'application/json': { schema: discoveryInputSchema } },
      },
      responses: {
        200: jsonResponse(discoveryInputSchema),
        400: badRequestResponse,
        401: unauthorizedResponse,
      },
    },
  }),
];
