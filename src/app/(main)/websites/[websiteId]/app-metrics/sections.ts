export const reportGroups = {
  Overview: ['overview'],
  Customers: ['acquisition', 'activation', 'retention', 'revenue'],
  Metrics: ['devices', 'versions', 'errors', 'localization', 'accessibility'],
  Explore: ['recent-events', 'event-types', 'parameters', 'playground'],
  Dashboards: ['dashboards'],
  Notebooks: ['notebooks'],
  Settings: ['settings'],
  Setup: ['setup'],
} as const;

export type AppReport = (typeof reportGroups)[keyof typeof reportGroups][number];

export function isAppReport(value: string): value is AppReport {
  return Object.values(reportGroups).some(group => group.some(slug => slug === value));
}
