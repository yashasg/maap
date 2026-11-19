export interface AllowlistEntry {
  /** Operation key, e.g. "POST /api/auth/sso" */
  key: string;
  /** Why this operation cannot be exercised by the harness. */
  reason: string;
  /** Only applies when running against this backend (UMAMI_TEST_DB); omit for both. */
  db?: 'postgres' | 'clickhouse';
}

/**
 * Operations that are intentionally not covered. Keep this list short and every
 * entry justified — the reporter warns when an allowlisted operation is
 * actually called, so entries can be removed once they gain a test.
 */
export const allowlist: AllowlistEntry[] = [
  {
    key: 'DELETE /api/websites/{websiteId}/sessions/{sessionId}',
    reason: 'Session deletion needs relational storage; sessions.spec.ts skips it on ClickHouse.',
    db: 'clickhouse',
  },
];
