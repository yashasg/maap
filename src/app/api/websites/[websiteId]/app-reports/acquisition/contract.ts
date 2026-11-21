import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { analyticsQuerySchema } from '../../analytics-schema';
import { appAcquisitionResponseSchema } from '../schemas';

const acquisitionQuerySchema = analyticsQuerySchema.extend({
  testMode: z.enum(['true', 'false']).optional(),
});

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/acquisition',
    audience: 'public',
    auth: 'bearer-or-share',
    operation: {
      operationId: 'getAppAcquisitionReport',
      summary: 'Get the app acquisition report',
      description:
        'Returns first-seen identified app users across time series, calendar patterns, and device, platform, region, and language breakdowns.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: acquisitionQuerySchema,
      },
      responses: {
        '200': jsonResponse(appAcquisitionResponseSchema, 'App acquisition report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
