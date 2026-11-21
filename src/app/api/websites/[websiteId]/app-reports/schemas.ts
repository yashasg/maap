import { z } from 'zod';

const countPointSchema = z.object({
  key: z.string(),
  label: z.string(),
  count: z.number(),
});

const acquisitionSeriesPointSchema = z.object({
  bucket: z.string(),
  activeUsers: z.number(),
  newUsers: z.number(),
});

const bucketCountSchema = z.object({
  bucket: z.string(),
  users: z.number(),
});

const namedCountSchema = z.object({
  name: z.string(),
  users: z.number(),
});

const lifecycleSeriesSchema = z.object({
  hourly: z.array(countPointSchema),
  daily: z.array(countPointSchema),
  weekly: z.array(countPointSchema),
  monthly: z.array(countPointSchema),
});

const lifecyclePatternSchema = z.object({
  hourOfDay: z.array(countPointSchema),
  hourOfDay4Weeks: z.array(countPointSchema),
  dayOfWeek: z.array(countPointSchema),
  dayOfWeek4Weeks: z.array(countPointSchema),
  weekpart: z.array(countPointSchema),
  weekpart4Weeks: z.array(countPointSchema),
});

const lifecycleBreakdownSchema = z.object({
  device: z.array(countPointSchema),
  platform: z.array(countPointSchema),
  region: z.array(countPointSchema),
  language: z.array(countPointSchema),
});

export const appOverviewResponseSchema = z
  .object({
    identity: z.literal('clientUser'),
    range: z.object({
      startAt: z.iso.datetime(),
      endAt: z.iso.datetime(),
      timezone: z.string(),
    }),
    summary: z.object({
      users: z.number(),
      usersLastHour: z.number(),
      events: z.number(),
      errors: z.number().nullable(),
      mostUsedSdk: z.string().nullable(),
    }),
    dailyUsers: z.array(z.object({ x: z.string(), y: z.number() })),
    retention: z.array(
      z.object({
        day: z.number(),
        eligibleUsers: z.number(),
        returningUsers: z.number(),
        percentage: z.number().nullable(),
      }),
    ),
    dimensions: z.object({
      sdkName: z.array(z.object({ value: z.string(), users: z.number() })),
      sdkVersion: z.array(z.object({ value: z.string(), users: z.number() })),
      appVersion: z.array(z.object({ value: z.string(), users: z.number() })),
      buildNumber: z.array(z.object({ value: z.string(), users: z.number() })),
      systemVersion: z.array(z.object({ value: z.string(), users: z.number() })),
    }),
  })
  .meta({ id: 'AppOverviewReport' });

export const appAcquisitionResponseSchema = z
  .object({
    identity: z.object({
      unit: z.literal('appUser'),
      source: z.literal('clientUser'),
      unidentifiedEventsExcluded: z.literal(true),
    }),
    range: z.object({
      startAt: z.iso.datetime(),
      endAt: z.iso.datetime(),
      timezone: z.string(),
      testMode: z.boolean(),
      totalSignals: z.number(),
      identifiedSignals: z.number(),
    }),
    series: z.object({
      hourly: z.array(acquisitionSeriesPointSchema),
      daily: z.array(acquisitionSeriesPointSchema),
      weekly: z.array(acquisitionSeriesPointSchema),
      monthly: z.array(acquisitionSeriesPointSchema),
    }),
    patterns: z.object({
      hourOfDay: z.array(bucketCountSchema),
      hourOfDay4Weeks: z.array(bucketCountSchema),
      dayOfWeek: z.array(bucketCountSchema),
      dayOfWeek4Weeks: z.array(bucketCountSchema),
      weekpart: z.array(bucketCountSchema),
      weekpart4Weeks: z.array(bucketCountSchema),
    }),
    breakdowns: z.object({
      device: z.array(namedCountSchema),
      platform: z.array(namedCountSchema),
      region: z.array(namedCountSchema),
      language: z.array(namedCountSchema),
    }),
    availability: z.object({
      userLocalTime: z.literal(false),
      reason: z.string(),
    }),
    query: z.object({
      milliseconds: z.number(),
      grains: z.object({
        hourly: z.object({ unit: z.literal('hour'), window: z.literal('last24Hours') }),
        daily: z.object({ unit: z.literal('day'), window: z.literal('selectedRange') }),
        weekly: z.object({
          unit: z.literal('week'),
          window: z.literal('last3CalendarMonths'),
        }),
        monthly: z.object({
          unit: z.literal('month'),
          window: z.literal('last12CalendarMonths'),
        }),
      }),
    }),
  })
  .meta({ id: 'AppAcquisitionReport' });

export const appActivationResponseSchema = z
  .object({
    definition: z.object({
      rule: z.string(),
      thresholdSeconds: z.number(),
      sessionLimit: z.number(),
      source: z.string(),
      configurable: z.boolean(),
      calculation: z.string(),
    }),
    configuration: z.object({
      status: z.enum(['empty', 'identity-missing', 'configuration-missing', 'partial', 'ready']),
      coreReady: z.boolean(),
      missingKeys: z.array(z.string()),
      requiredAllowlist: z.array(z.string()),
      environmentVariable: z.string(),
      historicalBackfill: z.boolean(),
    }),
    scope: z.object({
      startAt: z.iso.datetime(),
      endAt: z.iso.datetime(),
      timezone: z.string(),
      testMode: z.boolean(),
      totalSignals: z.number(),
      identifiedSignals: z.number(),
      activationInputSignals: z.number(),
      activatedUsers: z.number(),
    }),
    series: lifecycleSeriesSchema,
    patterns: lifecyclePatternSchema,
    breakdowns: lifecycleBreakdownSchema,
    query: z.object({
      identity: z.literal('clientUser'),
      unidentifiedSignalsExcluded: z.literal(true),
    }),
  })
  .meta({ id: 'AppActivationReport' });

const retentionSeriesSchema = z.object({
  hourly: z.array(z.object({ bucket: z.string(), returningUsers: z.number() })),
  daily: z.array(z.object({ bucket: z.string(), returningUsers: z.number() })),
  weekly: z.array(z.object({ bucket: z.string(), returningUsers: z.number() })),
  monthly: z.array(z.object({ bucket: z.string(), returningUsers: z.number() })),
});

export const appRetentionResponseSchema = z
  .object({
    criteria: z.object({
      verified: z.boolean(),
      rule: z.string(),
      verification: z.string(),
    }),
    identity: z.object({
      unit: z.literal('appUser'),
      source: z.literal('clientUser'),
      unidentifiedEventsExcluded: z.literal(true),
    }),
    range: z.object({
      startAt: z.iso.datetime(),
      endAt: z.iso.datetime(),
      timezone: z.string(),
      testMode: z.boolean(),
    }),
    scope: z.object({
      totalSignals: z.number(),
      identifiedSignals: z.number(),
      returningUsers: z.number(),
    }),
    series: retentionSeriesSchema,
    patterns: z.object({
      hourOfDay: z.array(bucketCountSchema),
      hourOfDayFourWeeks: z.array(bucketCountSchema),
      dayOfWeek: z.array(bucketCountSchema),
      dayOfWeekFourWeeks: z.array(bucketCountSchema),
      weekpart: z.array(bucketCountSchema),
      weekpartFourWeeks: z.array(bucketCountSchema),
    }),
    breakdowns: z.object({
      device: z.array(namedCountSchema),
      platform: z.array(namedCountSchema),
      region: z.array(namedCountSchema),
      language: z.array(namedCountSchema),
    }),
    availability: z.object({
      userLocalTime: z.literal(false),
      preferredRegion: z.literal(false),
      reason: z.string(),
    }),
    query: z.object({
      grains: z.object({
        hourly: z.object({
          unit: z.literal('hour'),
          window: z.literal('last24HoursWithinSelectedRange'),
        }),
        daily: z.object({ unit: z.literal('day'), window: z.literal('selectedRange') }),
        weekly: z.object({
          unit: z.literal('week'),
          window: z.literal('last93DaysWithinSelectedRange'),
        }),
        monthly: z.object({
          unit: z.literal('month'),
          window: z.literal('last366DaysWithinSelectedRange'),
        }),
      }),
    }),
  })
  .meta({ id: 'AppRetentionReport' });

export const appDevicesResponseSchema = z
  .object({
    reports: z.array(
      z.object({
        id: z.enum([
          'models',
          'systemVersions',
          'types',
          'platforms',
          'screenWidths',
          'screenHeights',
          'colorSchemes',
          'orientations',
        ]),
        totalSignals: z.number(),
        reportedSignals: z.number(),
        values: z.array(z.object({ value: z.string().nullable(), signals: z.number() })),
      }),
    ),
  })
  .meta({ id: 'AppDevicesReport' });

export const appVersionsResponseSchema = z
  .object({
    dimension: z.enum(['appVersion', 'buildNumber', 'sdkVersion']),
    totalEvents: z.number(),
    reportedEvents: z.number(),
    values: z.array(z.object({ value: z.string().nullable(), events: z.number() })),
  })
  .meta({ id: 'AppVersionsReport' });

export const appAccessibilityResponseSchema = z
  .object({
    reports: z.array(
      z.object({
        id: z.enum([
          'preferredContentSize',
          'boldText',
          'reduceMotion',
          'reduceTransparency',
          'darkerSystemColors',
          'differentiateWithoutColor',
          'invertedColors',
        ]),
        totalSignals: z.number(),
        reportedSignals: z.number(),
        values: z.array(z.object({ label: z.string(), signals: z.number() })),
      }),
    ),
  })
  .meta({ id: 'AppAccessibilityReport' });

export const appEventTypesResponseSchema = z
  .object({
    data: z.array(
      z.object({
        name: z.string(),
        users: z.number(),
        events: z.number(),
      }),
    ),
  })
  .meta({ id: 'AppEventTypesReport' });
