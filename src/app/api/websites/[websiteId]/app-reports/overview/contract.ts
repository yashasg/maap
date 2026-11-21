import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appOverviewResponseSchema } from '../schemas';

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/overview',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'getAppOverviewReport',
      summary: 'Get the app overview report',
      description:
        'Returns identified-user totals, daily activity, retention checkpoints, and current app and SDK dimension breakdowns for the selected scope.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: analyticsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appOverviewResponseSchema, 'App overview report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
