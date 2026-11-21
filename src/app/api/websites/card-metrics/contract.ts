import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';

const timestamp = z.number().int().min(0).max(8_640_000_000_000_000);
export const cardMetricsInput = z.strictObject({
  websiteIds: z.array(z.uuid()).min(1).max(20),
  startAt: timestamp,
  endAt: timestamp,
  testMode: z.boolean(),
  teamId: z.uuid().optional(),
});

export const operations = [
  defineOperation({
    path: '/api/websites/card-metrics',
    method: 'post',
    audience: 'internal',
    auth: 'bearer',
    operation: {
      operationId: 'postWebsitesCardMetrics',
      summary: 'Get user and error counts for workspace cards',
      description:
        'Supply distinct website IDs from one workspace and an ordered date range of at most 366 days. Counts are returned in the requested order.',
      tags: ['Workspace'],
      requestBody: {
        required: true,
        content: { 'application/json': { schema: cardMetricsInput } },
      },
      responses: {
        200: jsonResponse(
          z.object({
            data: z.array(
              z.object({
                websiteId: z.uuid(),
                users: z.int().nonnegative(),
                errors: z.int().nonnegative(),
              }),
            ),
          }),
        ),
        400: badRequestResponse,
        401: unauthorizedResponse,
        500: serverErrorResponse,
      },
    },
  }),
];
