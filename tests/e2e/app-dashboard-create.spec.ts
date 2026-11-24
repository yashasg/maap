import { randomUUID } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { authHeaders, deleteWebsite, loginPage } from './helpers';

test('creates an app dashboard from its visible trigger and reopens it after reload', async ({
  page,
  request,
}) => {
  const auth = await loginPage(page, request);
  const headers = authHeaders(auth);
  const created = await request.post('/api/websites', {
    headers,
    data: { name: `Dashboard fixture ${randomUUID()}`, domain: 'app.ios' },
  });
  expect(created.status()).toBe(200);
  const { id } = await created.json();
  let boardId: string | undefined;
  try {
    await page.goto(`/websites/${id}/app-reports/dashboards?testMode=true`);
    const newDashboard = page.getByRole('button', { name: 'New Dashboard', exact: true });
    const dialog = page.getByRole('dialog');
    await newDashboard.click();
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await newDashboard.click();
    await dialog.getByRole('textbox', { name: 'Name' }).fill('Release health');
    const saved = page.waitForResponse(
      r => new URL(r.url()).pathname === '/api/boards' && r.request().method() === 'POST',
    );
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();
    const response = await saved;
    expect(response.status()).toBe(200);
    boardId = (await response.json()).id;
    await expect(page.getByRole('heading', { name: 'Release health' })).toBeVisible();
    expect(new URL(page.url()).searchParams.get('testMode')).toBe('true');
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Release health' })).toBeVisible();
    await page.getByRole('button', { name: 'Import Dashboard', exact: true }).click();
    await dialog.getByLabel('Dashboard export file (.json)').setInputFiles({
      name: 'invalid.json',
      mimeType: 'application/json',
      buffer: Buffer.from('{}'),
    });
    await dialog.getByRole('button', { name: 'Import', exact: true }).click();
    await expect(dialog.getByRole('alert')).toContainText('version 1 Maap Board');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    const appDashboardUrl = page.url();
    await page.getByText('Design dashboard', { exact: true }).click();
    await page.getByRole('button', { name: 'Select component', exact: true }).click();
    await dialog.getByText('Events chart', { exact: true }).click();
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    const changed = page.waitForResponse(
      r =>
        new URL(r.url()).pathname === `/api/boards/${boardId}` && r.request().method() === 'POST',
    );
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    expect((await changed).status()).toBe(200);
    await page.waitForURL(url => url.pathname === `/boards/${boardId}`);
    await page.goto(appDashboardUrl);
    await expect(page.getByRole('heading', { name: 'Events chart', exact: true })).toBeVisible();
  } finally {
    if (boardId) await request.delete(`/api/boards/${boardId}`, { headers });
    await deleteWebsite(request, auth, id);
  }
});
