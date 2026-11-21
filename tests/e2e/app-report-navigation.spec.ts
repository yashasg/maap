import { randomUUID } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { authHeaders, deleteWebsite, loginPage } from './helpers';

test('a native app can navigate every Metrics report without losing Test Mode', async ({
  page,
  request,
}) => {
  const auth = await loginPage(page, request);
  const created = await request.post('/api/websites', {
    headers: authHeaders(auth),
    data: { name: `Metrics ${randomUUID()}`, domain: 'app.ios' },
  });
  expect(created.status()).toBe(200);
  const { id } = await created.json();
  try {
    const root = `/websites/${id}/app-reports`;
    await page.goto(`${root}/versions?testMode=true&date=30day`);
    const sidebar = page.getByRole('navigation', { name: 'Metrics reports' });
    for (const name of ['Devices', 'Errors', 'Localization', 'Accessibility', 'Versions']) {
      const link = sidebar.getByRole('link', { name, exact: true });
      await link.click();
      await expect(link).toHaveAttribute('aria-current', 'page');
      await expect(page.getByRole('heading', { name, exact: true }).last()).toBeVisible();
      const url = new URL(page.url());
      expect(url.pathname).toBe(`${root}/${name.toLowerCase()}`);
      expect(url.searchParams.get('testMode')).toBe('true');
    }
  } finally {
    await deleteWebsite(request, auth, id);
  }
});
