import { describe, expect, test } from 'vitest';
import { parseMaapBoardImport } from './Screen';

describe('parseMaapBoardImport', () => {
  test('keeps only supported Maap Board fields and regenerates layout ids', () => {
    const result = parseMaapBoardImport({
      format: 'maap-board',
      version: 1,
      name: 'Release health',
      description: 'Signals for this app',
      rows: [
        {
          id: 'untrusted-row',
          columns: [
            {
              id: 'untrusted-column',
              component: {
                type: 'WebsiteChart',
                websiteId: '00000000-0000-4000-8000-000000000000',
                entityType: 'website',
                entityId: '00000000-0000-4000-8000-000000000000',
                unknown: 'discarded',
              },
            },
          ],
        },
      ],
    });

    expect(result).toMatchObject({
      name: 'Release health',
      description: 'Signals for this app',
      parameters: {
        rows: [
          {
            columns: [{ component: { type: 'WebsiteChart' } }],
          },
        ],
      },
    });
    expect(result.parameters.rows?.[0].id).not.toBe('untrusted-row');
    expect(result.parameters.rows?.[0].columns[0].id).not.toBe('untrusted-column');
    expect(result.parameters.rows?.[0].columns[0].component).not.toHaveProperty('websiteId');
    expect(result.parameters.rows?.[0].columns[0].component).not.toHaveProperty('entityId');
  });

  test.each([
    [{ format: 'telemetrydeck-dashboard', version: 1, name: 'No', rows: [] }],
    [{ format: 'maap-board', version: 1, name: 'No', rows: [{ columns: [] }] }],
    [
      {
        format: 'maap-board',
        version: 1,
        name: 'No',
        rows: [{ columns: [{ component: { type: 'UnknownComponent' } }] }],
      },
    ],
  ])('rejects unsupported or malformed imports', value => {
    expect(() => parseMaapBoardImport(value)).toThrow();
  });
});
