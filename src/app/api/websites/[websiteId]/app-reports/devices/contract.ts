import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appDevicesResponseSchema } from '../schemas';

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/devices',
    audience: 'public',
    auth: 'bearer-or-share',
    operation: {
      operationId: 'getAppDevicesReport',
      summary: 'Get the app devices report',
      description:
        'Returns aggregate signal counts for device models, operating systems, platforms, screen sizes, color schemes, and orientations.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: analyticsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appDevicesResponseSchema, 'App devices report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
