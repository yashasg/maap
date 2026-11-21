import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appActivationResponseSchema } from '../schemas';

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/activation',
    audience: 'public',
    auth: 'bearer',
    operation: {
      operationId: 'getAppActivationReport',
      summary: 'Get the app activation report',
      description:
        'Returns identified users that meet the fixed five-minute activation rule, along with input availability, time series, calendar patterns, and breakdowns.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: analyticsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appActivationResponseSchema, 'App activation report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
