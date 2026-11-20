export const OVERVIEW_LAYOUTS = ['application', 'website'] as const;
export const APP_COLOR_SCHEMES = ['default', 'vibrant', 'high-contrast', 'emo'] as const;
export const REVENUE_DISPLAY_MODES = ['none', 'revenuecat'] as const;

export type OverviewLayout = (typeof OVERVIEW_LAYOUTS)[number];
export type AppColorScheme = (typeof APP_COLOR_SCHEMES)[number];
export type RevenueDisplayMode = (typeof REVENUE_DISPLAY_MODES)[number];

export type AppSettings = {
  overviewLayout: OverviewLayout;
  colorScheme: AppColorScheme;
  revenueDisplayMode: RevenueDisplayMode;
};

export type AppSettingsResponse = AppSettings & {
  websiteId: string;
  canWrite: boolean;
};

export const DEFAULT_APP_SETTINGS: AppSettings = {
  overviewLayout: 'application',
  colorScheme: 'default',
  revenueDisplayMode: 'none',
};

export const APP_COLOR_PALETTES: Record<AppColorScheme, readonly string[]> = {
  default: [
    '#ee8b72',
    '#f4bd38',
    '#9edb3a',
    '#45c9a7',
    '#3cc5d8',
    '#55b3de',
    '#76a7e5',
    '#cf78df',
    '#eb7892',
  ],
  vibrant: ['#f47f68', '#f7b927', '#dfe923', '#219bc4', '#277c9f', '#bd654e', '#d452c3', '#e35e89'],
  'high-contrast': [
    '#ef806a',
    '#2385ef',
    '#43bd68',
    '#6c5bd3',
    '#ee3f72',
    '#a64fc6',
    '#52b6d8',
    '#ffc51b',
  ],
  emo: ['#29292d', '#202024', '#5d5d66', '#e43f3f'],
};
