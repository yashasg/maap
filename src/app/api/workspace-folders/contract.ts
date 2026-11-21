import { z } from 'zod';
import { defineOperation } from '@/openapi/operation';
import {
  badRequestResponse,
  jsonResponse,
  okSchema,
  unauthorizedResponse,
} from '@/openapi/schemas';

export const folderScope = z.object({ teamId: z.uuid().optional() });
export const createFolder = folderScope.extend({ name: z.string().trim().min(1).max(100) });
export const moveWebsite = folderScope.extend({
  websiteId: z.uuid(),
  folderId: z.uuid().nullable(),
});
const folder = z.object({ id: z.uuid(), name: z.string() });
const assignment = z.object({ websiteId: z.uuid(), folderId: z.uuid() });
const list = z.object({ folders: z.array(folder), assignments: z.array(assignment) });
const scope = { path: '/api/workspace-folders', audience: 'internal', auth: 'bearer' } as const;
const errors = { 400: badRequestResponse, 401: unauthorizedResponse };

export const operations = [
  defineOperation({
    ...scope,
    method: 'get',
    operation: {
      operationId: 'getWorkspaceFolders',
      summary: 'List folders and website assignments in the workspace',
      tags: ['Workspace'],
      requestParams: { query: folderScope },
      responses: { 200: jsonResponse(list), ...errors },
    },
  }),
  defineOperation({
    ...scope,
    method: 'post',
    operation: {
      operationId: 'postWorkspaceFolders',
      summary: 'Create a folder in the workspace',
      tags: ['Workspace'],
      requestBody: { required: true, content: { 'application/json': { schema: createFolder } } },
      responses: { 200: jsonResponse(folder), ...errors },
    },
  }),
  defineOperation({
    ...scope,
    method: 'put',
    operation: {
      operationId: 'putWorkspaceFolders',
      summary: 'Move a website to a folder, or to the workspace root',
      tags: ['Workspace'],
      requestBody: { required: true, content: { 'application/json': { schema: moveWebsite } } },
      responses: { 200: jsonResponse(okSchema), ...errors },
    },
  }),
];
