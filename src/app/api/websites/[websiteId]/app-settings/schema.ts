import { z } from 'zod';
import { APP_COLOR_SCHEMES, OVERVIEW_LAYOUTS, REVENUE_DISPLAY_MODES } from '@/lib/app-settings';

export const appSettingsInputSchema = z
  .object({
    overviewLayout: z.enum(OVERVIEW_LAYOUTS),
    colorScheme: z.enum(APP_COLOR_SCHEMES),
    revenueDisplayMode: z.enum(REVENUE_DISPLAY_MODES),
  })
  .strict();
