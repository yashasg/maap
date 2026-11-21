import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  forbiddenResponse,
  jsonResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { notebookInputSchema, notebookListResponseSchema, notebookSchema } from './schema';

const websitePathSchema = z.object({
  websiteId: z.string(),
});

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/notebooks',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'listWebsiteNotebooks',
      summary: 'List website notebooks',
      description:
        'Returns notebook summaries for the website and whether the authenticated user can edit them.',
      tags: ['Websites'],
      requestParams: { path: websitePathSchema },
      responses: {
        '200': jsonResponse(notebookListResponseSchema, 'Website notebooks.'),
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
      },
    },
  }),
  defineOperation({
    method: 'post',
    path: '/api/websites/{websiteId}/notebooks',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'createWebsiteNotebook',
      summary: 'Create a website notebook',
      description: 'Creates a notebook for a website the authenticated user can edit.',
      tags: ['Websites'],
      requestParams: { path: websitePathSchema },
      requestBody: {
        required: true,
        content: {
          'application/json': {
            schema: notebookInputSchema,
          },
        },
      },
      responses: {
        '200': jsonResponse(notebookSchema, 'Notebook created.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
      },
    },
  }),
] as const;
