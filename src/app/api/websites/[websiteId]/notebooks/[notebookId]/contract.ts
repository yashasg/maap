import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  forbiddenResponse,
  jsonResponse,
  notFoundResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { notebookInputSchema, notebookResponseSchema, notebookSchema } from '../schema';

const notebookPathSchema = z.object({
  websiteId: z.string(),
  notebookId: z.string(),
});

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/notebooks/{notebookId}',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'getWebsiteNotebook',
      summary: 'Get a website notebook',
      description:
        'Returns a notebook from the website and whether the authenticated user can edit it.',
      tags: ['Websites'],
      requestParams: { path: notebookPathSchema },
      responses: {
        '200': jsonResponse(notebookResponseSchema, 'Website notebook.'),
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
        '404': notFoundResponse,
      },
    },
  }),
  defineOperation({
    method: 'post',
    path: '/api/websites/{websiteId}/notebooks/{notebookId}',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'updateWebsiteNotebook',
      summary: 'Update a website notebook',
      description: 'Replaces the title and content of a notebook in the specified website.',
      tags: ['Websites'],
      requestParams: { path: notebookPathSchema },
      requestBody: {
        required: true,
        content: {
          'application/json': {
            schema: notebookInputSchema,
          },
        },
      },
      responses: {
        '200': jsonResponse(notebookSchema, 'Notebook updated.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '403': forbiddenResponse,
        '404': notFoundResponse,
      },
    },
  }),
] as const;
