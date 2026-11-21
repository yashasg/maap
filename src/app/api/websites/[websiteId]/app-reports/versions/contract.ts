import { z } from 'zod';
import { withDateRange } from '@/lib/schema';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  serverErrorResponse,
  unauthorizedResponse,
} from '@/openapi/schemas';
import { websiteIdPathSchema } from '../../../response-schema';
import { appVersionsResponseSchema } from '../schemas';

const versionsQuerySchema = withDateRange({
  dimension: z.enum(['appVersion', 'buildNumber', 'sdkVersion']),
  tag: z.enum(['test', 'eq.test', 'neq.test']).optional(),
  testMode: z.enum(['true', 'false']).optional(),
}).superRefine((data, context) => {
  if (data.tag !== undefined && data.testMode !== undefined) {
    context.addIssue({
      code: z.ZodIssueCode.custom,
      message: 'testMode cannot be combined with tag',
    });
  }
});

export const operations = [
  defineOperation({
    method: 'get',
    path: '/api/websites/{websiteId}/app-reports/versions',
    audience: 'public',
    auth: 'bearer-or-share',
    operation: {
      operationId: 'getAppVersionsReport',
      summary: 'Get the app versions report',
      description:
        'Returns event counts grouped by app version, build number, or SDK version for the selected scope.',
      tags: ['App Reports'],
      requestParams: {
        path: websiteIdPathSchema,
        query: versionsQuerySchema,
      },
      responses: {
        '200': jsonResponse(appVersionsResponseSchema, 'App versions report.'),
        '400': badRequestResponse,
        '401': unauthorizedResponse,
        '500': serverErrorResponse,
      },
    },
  }),
] as const;
