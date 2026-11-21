import { expect, test } from './fixtures';
import { dateRange, TIMEZONE } from './helpers/dates';
import { createWebsite, deleteWebsite, uniqueName } from './helpers/entities';

test('GET app report endpoints return data for an authorized user', async ({ admin, seed }) => {
  const base = `/api/websites/${seed.website.id}/app-reports`;
  const params = { ...dateRange(seed), timezone: TIMEZONE };
  const reports = [
    'accessibility',
    'activation',
    'devices',
    'event-types',
    'overview',
    'retention',
  ];

  for (const report of reports) {
    const response = await admin.get(`${base}/${report}`, { params });
    expect(response.status, `${report}: ${response.text}`).toBe(200);
  }

  const versions = await admin.get(`${base}/versions`, {
    params: { ...params, dimension: 'appVersion' },
  });
  expect(versions.status, `versions: ${versions.text}`).toBe(200);
});

test('workspace APIs persist app settings, notebooks, folders, metrics, and discovery', async ({
  admin,
  seed,
}) => {
  const website = await createWebsite(admin, { name: uniqueName('app-workspace') });
  const base = `/api/websites/${website.id}`;

  try {
    const discovery = await admin.get('/api/me/discovery');
    expect(discovery.status).toBe(200);

    const savedDiscovery = await admin.post('/api/me/discovery', { choice: 'recommendation' });
    expect(savedDiscovery.status).toBe(200);
    expect(savedDiscovery.body).toEqual({ choice: 'recommendation' });

    const settings = await admin.get(`${base}/app-settings`);
    expect(settings.status).toBe(200);
    expect(settings.body).toMatchObject({ websiteId: website.id, canWrite: true });

    const updatedSettings = await admin.post(`${base}/app-settings`, {
      overviewLayout: 'application',
      colorScheme: 'vibrant',
      revenueDisplayMode: 'none',
    });
    expect(updatedSettings.status).toBe(200);
    expect(updatedSettings.body).toMatchObject({
      websiteId: website.id,
      overviewLayout: 'application',
      colorScheme: 'vibrant',
      revenueDisplayMode: 'none',
      canWrite: true,
    });

    const notebooks = await admin.get(`${base}/notebooks`);
    expect(notebooks.status).toBe(200);
    expect(notebooks.body).toMatchObject({ data: [], canWrite: true });

    const createdNotebook = await admin.post(`${base}/notebooks`, {
      title: 'Release notes',
      content: {
        version: 1,
        blocks: [{ id: 'summary', type: 'paragraph', text: 'Initial findings' }],
      },
    });
    expect(createdNotebook.status).toBe(200);
    expect(createdNotebook.body).toMatchObject({
      websiteId: website.id,
      title: 'Release notes',
    });

    const notebookPath = `${base}/notebooks/${createdNotebook.body.id}`;
    const notebook = await admin.get(notebookPath);
    expect(notebook.status).toBe(200);
    expect(notebook.body).toMatchObject({
      data: { id: createdNotebook.body.id, title: 'Release notes' },
      canWrite: true,
    });

    const updatedNotebook = await admin.post(notebookPath, {
      title: 'Release review',
      content: {
        version: 1,
        blocks: [{ id: 'summary', type: 'paragraph', text: 'Reviewed findings' }],
      },
    });
    expect(updatedNotebook.status).toBe(200);
    expect(updatedNotebook.body).toMatchObject({
      id: createdNotebook.body.id,
      title: 'Release review',
    });

    const metrics = await admin.post('/api/websites/card-metrics', {
      websiteIds: [website.id],
      startAt: seed.range.startAt,
      endAt: Date.now(),
      testMode: false,
    });
    expect(metrics.status).toBe(200);
    expect(metrics.body.data).toEqual([
      expect.objectContaining({ websiteId: website.id, users: 0, errors: 0 }),
    ]);

    const folders = await admin.get('/api/workspace-folders');
    expect(folders.status).toBe(200);

    const createdFolder = await admin.post('/api/workspace-folders', {
      name: uniqueName('release'),
    });
    expect(createdFolder.status).toBe(200);

    const assigned = await admin.put('/api/workspace-folders', {
      websiteId: website.id,
      folderId: createdFolder.body.id,
    });
    expect(assigned.status).toBe(200);

    const assignedFolders = await admin.get('/api/workspace-folders');
    expect(assignedFolders.status).toBe(200);
    expect(assignedFolders.body.assignments).toContainEqual({
      websiteId: website.id,
      folderId: createdFolder.body.id,
    });

    const movedToRoot = await admin.put('/api/workspace-folders', {
      websiteId: website.id,
      folderId: null,
    });
    expect(movedToRoot.status).toBe(200);
  } finally {
    await deleteWebsite(admin, website.id);
  }
});
