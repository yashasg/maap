import { z } from 'zod';

export const notebookContentSchema = z.object({
  version: z.literal(1),
  blocks: z
    .array(
      z
        .object({
          id: z.string().min(1).max(64),
          type: z.enum(['heading', 'paragraph', 'checklist']),
          text: z.string().max(10000),
          checked: z.boolean().optional(),
        })
        .refine(block => block.type === 'checklist' || block.checked === undefined, {
          message: 'Only checklist blocks can be checked.',
        }),
    )
    .min(1)
    .max(200)
    .refine(blocks => new Set(blocks.map(block => block.id)).size === blocks.length, {
      message: 'Block IDs must be unique.',
    }),
});

export const notebookInputSchema = z.object({
  title: z.string().trim().min(1).max(200),
  content: notebookContentSchema,
});
