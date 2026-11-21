import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appEventTypesResponseSchema } from '../schemas';

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/event-types',
    audience: 'public',
    auth: 'bearer-or-share',
    operation: {
      operationId: 'getAppEventTypesReport',
      summary: 'Get the app event types report',
      description:
        'Returns custom event names with total event counts and distinct identified-user counts for the selected scope.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: analyticsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appEventTypesResponseSchema, 'App event types report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
