import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appAccessibilityResponseSchema } from '../schemas';

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/accessibility',
    audience: 'public',
    auth: 'bearer-or-share',
    operation: {
      operationId: 'getAppAccessibilityReport',
      summary: 'Get the app accessibility report',
      description:
        'Returns aggregate signal counts for reported Dynamic Type and accessibility preference values without exposing individual users.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: analyticsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appAccessibilityResponseSchema, 'App accessibility report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
