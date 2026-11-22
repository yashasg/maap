import { randomUUID } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { authHeaders, deleteWebsite, loginPage } from './helpers';

test('static-shell tabs and table refreshes never reload or discard loaded content', async ({
  page,
}) => {
  const id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
  const root = `/websites/${id}`;
  const reports = `${root}/app-reports`;
  let release: (() => void) | undefined;
  let delayEvents = false;
  let eventRequests = 0;
  const documentRequests: string[] = [];
  const rscRequests: string[] = [];
  page.on('request', request => {
    if (request.isNavigationRequest()) documentRequests.push(request.url());
    if (request.headers().rsc) rscRequests.push(request.url());
  });
  // Match Rust's absent RSC endpoint, without requiring a database.
  await page.route('**/*', async route => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.headers().rsc) return route.fulfill({ status: 404 });
    if (!path.startsWith('/api/')) return route.continue();
    let json: unknown = {};
    if (path === '/api/auth/verify') json = { id: 'reader', username: 'reader', role: 'user' };
    else if (path === `/api/websites/${id}`) {
      json = { id, name: 'Navigation fixture', domain: 'app.ios' };
    } else if (path.endsWith('/app-reports/overview')) {
      json = { summary: { users: 12, mostUsedSdk: 'Swift', errors: null } };
    } else if (path.endsWith('/events')) {
      eventRequests++;
      if (delayEvents) await new Promise<void>(resolve => (release = resolve));
      json = {
        data: [
          {
            id: 'event-1',
            sessionId: 'session-1',
            createdAt: new Date().toISOString(),
            eventName: 'Checkout opened',
            urlPath: '/checkout',
            country: 'US',
            os: 'iOS',
            browser: 'Safari',
          },
        ],
        count: 1,
        page: 1,
        pageSize: 20,
      };
    } else if (path.endsWith('/event-data/fields')) json = [];
    else if (path.endsWith('/websites') || path.endsWith('/teams')) {
      json = { data: [], count: 0, page: 1, pageSize: 20 };
    }
    return route.fulfill({ json });
  });

  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await Promise.all([page.waitForResponse('**/api/auth/verify'), page.goto('/websites')]);
    // Rust serves this same entry point at property URLs.
    await page.evaluate(
      url => window.history.replaceState(null, '', url),
      `${reports}/recent-events?date=30day&testMode=true`,
    );
    await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();
    const header = await page.getByRole('main').locator('header').first().elementHandle();
    const documentsBefore = documentRequests.length;
    rscRequests.length = 0;
    const sidebar = page.getByRole('navigation', { name: 'Explore reports' });
    await sidebar.getByRole('link', { name: 'Parameters', exact: true }).click();
    await expect(page).toHaveURL(`${reports}/parameters?date=30day&testMode=true`);
    await expect(sidebar.getByRole('link', { name: 'Parameters', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(documentRequests).toHaveLength(documentsBefore);
    expect(rscRequests).toEqual([]);
    expect(await header?.evaluate(node => node.isConnected)).toBe(true);
    await page.goBack();
    await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();
    await page.goForward();
    await expect(page).toHaveURL(`${reports}/parameters?date=30day&testMode=true`);
    await sidebar.getByRole('link', { name: 'Recent Events', exact: true }).click();
    await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();
    const tabs = page.getByRole('navigation', { name: 'App reports', exact: true });
    await tabs.getByRole('link', { name: 'Setup', exact: true }).click();
    await expect(page).toHaveURL(`${reports}/setup?date=30day&testMode=true`);
    await tabs.getByRole('link', { name: 'Explore', exact: true }).click();
    await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();

    delayEvents = true;
    const requestsBefore = eventRequests;
    const cell = await page.getByText('Checkout opened', { exact: true }).elementHandle();
    try {
      await page.getByRole('button', { name: 'Refresh', exact: true }).click();
      await expect.poll(() => eventRequests).toBeGreaterThan(requestsBefore);
      await expect(page.getByRole('button', { name: 'Refreshing', exact: true })).toBeVisible();
      await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();
      expect(await cell?.evaluate(node => node.isConnected)).toBe(true);
    } finally {
      delayEvents = false;
      release?.();
      release = undefined;
    }
    await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeVisible();
    delayEvents = true;
    const searchRequests = eventRequests;
    const search = page.getByRole('searchbox');
    try {
      await search.fill('checkout');
      await expect.poll(() => eventRequests).toBeGreaterThan(searchRequests);
      await expect(page.getByText('Checkout opened', { exact: true })).toBeVisible();
      await expect(search).toBeFocused();
      await expect(search).toHaveValue('checkout');
    } finally {
      delayEvents = false;
      release?.();
      release = undefined;
    }
    await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeVisible();
    await search.clear();
    await expect(page).toHaveURL(`${reports}/recent-events?date=30day&testMode=true&page=1`);
    await page.getByRole('button', { name: 'Test Mode', exact: true }).click();
    await expect(page).toHaveURL(`${reports}/recent-events?date=30day`);
    expect(documentRequests).toHaveLength(documentsBefore);
    expect(rscRequests).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: test.info().outputPath(`navigation-${width}.png`) });
  }
});

test('websites use the shared report shell on home, deep links, settings and mobile', async ({
  page,
  request,
}) => {
  const auth = await loginPage(page, request);
  const ids: string[] = [];
  try {
    for (const domain of ['example.org', 'app.ios']) {
      const response = await request.post('/api/websites', {
        headers: authHeaders(auth),
        data: { name: `Shared UI ${domain} ${randomUUID()}`, domain },
      });
      expect(response.status()).toBe(200);
      ids.push((await response.json()).id);
    }
    const [websiteId, appId] = ids;
    const root = `/websites/${websiteId}`;
    const appRequests: string[] = [];
    page.on('request', request => {
      if (request.url().includes(`/api/websites/${websiteId}/app-reports/`)) {
        appRequests.push(request.url());
      }
    });
    await page.goto(`${root}?date=30day&testMode=true`);
    const header = page.getByRole('main').locator('header').first();
    await expect(header.getByText('Visitors', { exact: true })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Maap', exact: true })).toBeVisible();
    const shellClass = await header.locator('..').getAttribute('class');
    const traffic = page.getByRole('navigation', { name: 'Traffic reports' });
    await expect(traffic.getByRole('link', { name: 'Overview', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await traffic.getByRole('link', { name: 'Events', exact: true }).click();
    await expect(page).toHaveURL(`${root}/events?date=30day&testMode=true`);
    await expect(traffic.getByRole('link', { name: 'Events', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    await expect(page.getByRole('button', { name: 'Test Mode', exact: true })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await page.getByRole('button', { name: 'Test Mode', exact: true }).click();
    await expect(page).toHaveURL(`${root}/events?date=30day`);
    await page.reload();
    await expect(header.getByText('Visitors', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Test Mode', exact: true })).toHaveAttribute(
      'aria-pressed',
      'false',
    );

    const tabs = page.getByRole('navigation', { name: 'Reports', exact: true });
    await tabs.getByRole('link', { name: 'Settings', exact: true }).click();
    await expect(page).toHaveURL(`${root}/settings?date=30day`);
    await expect(tabs.getByRole('link', { name: 'Settings', exact: true })).toHaveAttribute(
      'aria-current',
      'location',
    );
    await expect(page.getByRole('button', { name: 'Test Mode', exact: true })).toHaveCount(0);

    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${root}/sessions?date=30day`);
    await expect(traffic.getByRole('link', { name: 'Sessions', exact: true })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    const pickerBox = await page.getByRole('combobox').first().boundingBox();
    const accountBox = await page.getByTestId('button-profile').boundingBox();
    expect(pickerBox).not.toBeNull();
    expect(accountBox).not.toBeNull();
    expect(accountBox!.x).toBeGreaterThanOrEqual(pickerBox!.x + pickerBox!.width);
    expect(appRequests).toEqual([]);

    await page.goto(`/websites/${appId}/app-reports/versions?date=30day`);
    await expect(header.getByText('Most used SDK', { exact: true })).toBeVisible();
    await expect(header.locator('..')).toHaveAttribute('class', shellClass!);
    await expect(page.getByRole('navigation', { name: 'Metrics reports' })).toBeVisible();
  } finally {
    for (const id of ids) await deleteWebsite(request, auth, id);
  }
});

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
    const propertyRoute = `**/api/websites/${id}`;
    let release!: () => void;
    const pending = new Promise<void>(resolve => {
      release = resolve;
    });
    await page.route(propertyRoute, async route => {
      await pending;
      await route.continue();
    });
    try {
      await Promise.all([
        page.waitForRequest(propertyRoute),
        page.goto(`${root}/versions?testMode=true&date=30day`, { waitUntil: 'domcontentloaded' }),
      ]);
      await expect(page.locator('[data-slot="loading"]')).toBeVisible();
      await expect(page.locator(`a[href*="/websites/${id}/events"]`)).toHaveCount(0);
    } finally {
      release();
    }
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
