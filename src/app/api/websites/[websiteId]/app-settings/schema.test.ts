import { expect, test } from 'vitest';
import { appSettingsInputSchema } from './schema';

test('accepts only the documented app preference options', () => {
  expect(
    appSettingsInputSchema.safeParse({
      overviewLayout: 'application',
      colorScheme: 'emo',
      revenueDisplayMode: 'revenuecat',
    }).success,
  ).toBe(true);

  for (const input of [
    {
      overviewLayout: 'dashboard',
      colorScheme: 'default',
      revenueDisplayMode: 'none',
    },
    {
      overviewLayout: 'application',
      colorScheme: 'custom',
      revenueDisplayMode: 'none',
    },
    {
      overviewLayout: 'application',
      colorScheme: 'default',
      revenueDisplayMode: 'stripe',
    },
  ]) {
    expect(appSettingsInputSchema.safeParse(input).success).toBe(false);
  }
});
