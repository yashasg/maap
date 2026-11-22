import { expect, type Page, test } from '@playwright/test';

const id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const root = `/websites/${id}`;
const reports = `${root}/app-reports`;

// Controlled local responses make the code-loading and data-loading frames observable separately.
async function fixture(page: Page, domain: string) {
  const gates = new Map<string, Promise<void>>();
  const requests: string[] = [];
  const documents: string[] = [];
  const rsc: string[] = [];
  const hold = (key: string) => {
    let release!: () => void;
    gates.set(
      key,
      new Promise<void>(resolve => {
        release = resolve;
      }),
    );
    return () => {
      gates.delete(key);
      release();
    };
  };
  page.on('request', request => {
    if (request.isNavigationRequest()) documents.push(request.url());
    // Global links still use Next; only report-local RSC requests violate the static-shell contract.
    if (request.headers().rsc && new URL(request.url()).pathname.startsWith(root)) {
      rsc.push(request.url());
    }
  });
  await page.route('**/*', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (request.headers().rsc) return route.fulfill({ status: 404 });
    if (request.resourceType() === 'script') await gates.get('code');
    if (!path.startsWith('/api/')) return route.continue();
    const key = path.endsWith('/metrics')
      ? url.searchParams.get('type')
      : path.endsWith('/versions')
        ? url.searchParams.get('dimension')
        : path.split('/').at(-1);
    if (!key) throw new Error(`Missing fixture request key: ${path}`);
    requests.push(key);
    await gates.get(key);
    let json: unknown = {};
    if (path === '/api/auth/verify') json = { id: 'reader', username: 'reader', role: 'user' };
    else if (path === `/api${root}`) json = { id, name: 'Transition fixture', domain };
    else if (['websites', 'teams', 'annotations', 'events'].includes(key)) {
      json = { data: [], count: 0, page: 1, pageSize: 20 };
    } else if (key === 'app-settings') {
      json = {
        overviewLayout: 'application',
        colorScheme: 'default',
        revenueDisplayMode: 'none',
        canWrite: false,
      };
    } else if (key === 'overview') {
      json = {
        range: { timezone: 'UTC' },
        summary: { users: 80, usersLastHour: 20, events: 100, mostUsedSdk: 'Swift', errors: null },
        dailyUsers: [{ x: '2026-11-01', y: 80 }],
        retention: [],
        dimensions: {
          sdkName: [],
          sdkVersion: [],
          appVersion: [],
          buildNumber: [],
          systemVersion: [],
        },
      };
    } else if (key === 'acquisition') {
      const points = [{ bucket: '2026-11-01', activeUsers: 80, newUsers: 20 }];
      json = {
        range: { timezone: 'UTC', totalSignals: 100, identifiedSignals: 80 },
        series: { hourly: points, daily: points, weekly: points, monthly: points },
        patterns: { hourOfDay4Weeks: [], dayOfWeek4Weeks: [], weekpart: [], weekpart4Weeks: [] },
        breakdowns: { device: [], platform: [], region: [], language: [] },
        availability: { userLocalTime: false },
        query: { milliseconds: 80 },
      };
    } else if (key === 'devices') {
      json = {
        reports: [
          'models',
          'systemVersions',
          'types',
          'platforms',
          'screenWidths',
          'screenHeights',
          'colorSchemes',
          'orientations',
        ].map(metric => ({
          id: metric,
          totalSignals: 100,
          reportedSignals: 100,
          values: [
            { value: 'Tablet', signals: 80 },
            { value: 'Phone', signals: 20 },
          ],
        })),
      };
    } else if (path.endsWith('/versions')) {
      json = {
        totalEvents: 100,
        reportedEvents: 100,
        values: [{ value: `${key}-ready`, events: 80 }],
      };
    } else if (path.endsWith('/metrics')) {
      const values =
        key === 'browser'
          ? ['chrome', 'firefox']
          : key === 'country'
            ? ['US', 'GB']
            : [`/${key}-first`, `/${key}-second`];
      json = [
        { x: values[0], y: 80 },
        { x: values[1], y: 20 },
      ];
    } else if (key === 'stats') {
      json = {
        visitors: 80,
        visits: 100,
        pageviews: 120,
        bounces: 10,
        totaltime: 6000,
        comparison: { visitors: 70, visits: 90, pageviews: 110, bounces: 10, totaltime: 5000 },
      };
    } else if (key === 'weekly' || key === 'series') json = [];
    else if (key === 'pageviews') json = { pageviews: [], sessions: [] };
    else if (key === 'active') json = { visitors: 1 };
    return route.fulfill({ json });
  });
  await page.goto('/websites');
  await expect(page.getByRole('main')).toBeVisible();
  await page.evaluate(
    url => window.history.replaceState(null, '', url),
    domain === 'app.ios' ? `${reports}/overview` : root,
  );
  return { hold, requests, documents, rsc };
}

type Frame = {
  height: number;
  panels: number;
  numbers: string[];
  percentages: string[];
  skeletons: number;
};
declare global {
  interface Window {
    transitionFrames: Frame[];
    recordTransitions: boolean;
  }
}

async function recordFrames(page: Page, selector: string) {
  await page.evaluate(selector => {
    window.transitionFrames = [];
    window.recordTransitions = true;
    const sample = () => {
      const root = document.querySelector(selector);
      const visible = (node: Element) => node.getBoundingClientRect().height > 0;
      if (root) {
        window.transitionFrames.push({
          height: root.getBoundingClientRect().height,
          panels: [...root.querySelectorAll('[data-slot="tabs-panel"]')].filter(visible).length,
          numbers: [...root.querySelectorAll('[title="80"]')]
            .filter(visible)
            .map(node => node.textContent ?? ''),
          percentages: [...root.querySelectorAll('[title="80"]')]
            .filter(visible)
            .map(node => node.closest('.grid')?.lastElementChild?.textContent ?? ''),
          skeletons: root.querySelectorAll('[data-report-skeleton]').length,
        });
      }
      if (window.recordTransitions) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  }, selector);
}

async function frames(page: Page) {
  // This observation window deliberately spans Base UI's exit and Motion's spring frames.
  await page.waitForTimeout(650);
  return page.evaluate(() => {
    window.recordTransitions = false;
    return window.transitionFrames;
  });
}

for (const width of [1440, 390]) {
  for (const reducedMotion of ['no-preference', 'reduce'] as const) {
    test(`table transitions keep one panel and factual values: ${width} ${reducedMotion}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion });
      const state = await fixture(page, 'example.org');
      const tabs = page
        .locator('[data-slot="tabs"]')
        .filter({ has: page.getByRole('tab', { name: 'Path', exact: true }) })
        .first();
      await tabs.evaluate(node => node.setAttribute('data-transition-test', 'pages'));
      const panel = tabs.getByRole('tabpanel');
      await expect(panel.getByTitle('80')).toHaveText('80');
      const originalHeight = await tabs.evaluate(node => node.getBoundingClientRect().height);
      const timeOrigin = await page.evaluate(() => performance.timeOrigin);
      const documents = state.documents.length;

      for (const [name, key] of [
        ['URL', 'fullPath'],
        ['Entry page', 'entry'],
        ['Exit page', 'exit'],
      ]) {
        const release = state.hold(key);
        try {
          await recordFrames(page, '[data-transition-test="pages"]');
          await tabs.getByRole('tab', { name, exact: true }).click();
          await expect.poll(() => state.requests.includes(key)).toBe(true);
          await expect(panel.locator('[data-report-skeleton="table"]')).toBeVisible();
          await expect(panel.locator('[data-slot="loading"]')).toHaveCount(0);
          await expect(panel.getByTitle('80')).toHaveCount(0);
          expect(await tabs.evaluate(node => node.getBoundingClientRect().height)).toBeCloseTo(
            originalHeight,
            0,
          );
          const animations = await panel
            .locator('[data-slot="skeleton"]')
            .evaluateAll(nodes => nodes.map(node => getComputedStyle(node).animationName));
          expect(animations.every(name => name === 'none')).toBe(true);
          if (key === 'entry')
            await page.screenshot({ path: test.info().outputPath('table-loading.png') });
          const coldFrames = await frames(page);
          expect(
            coldFrames.every(
              frame => frame.panels === 1 && Math.abs(frame.height - originalHeight) < 1,
            ),
          ).toBe(true);
          expect(coldFrames.every(frame => frame.numbers[0] === '80' || frame.skeletons > 0)).toBe(
            true,
          );
        } finally {
          release();
        }
        await expect(panel.getByTitle('80')).toHaveText('80');
      }

      const before = state.requests.length;
      await recordFrames(page, '[data-transition-test="pages"]');
      for (const name of ['Path', 'URL', 'Path', 'URL']) {
        await tabs.getByRole('tab', { name, exact: true }).click();
        await page.waitForTimeout(80);
      }
      // Exercise Base UI's keyboard activation and focus semantics, not a click-only substitute.
      await tabs.getByRole('tab', { name: 'URL', exact: true }).focus();
      await page.keyboard.press('ArrowLeft');
      await expect(tabs.getByRole('tab', { name: 'Path', exact: true })).toBeFocused();
      await page.keyboard.press('Enter');
      await expect(tabs.getByRole('tab', { name: 'Path', exact: true })).toHaveAttribute(
        'aria-selected',
        'true',
      );
      const samples = await frames(page);
      expect(samples.length).toBeGreaterThan(5);
      expect(samples.every(frame => frame.panels === 1)).toBe(true);
      expect(samples.every(frame => Math.abs(frame.height - originalHeight) < 1)).toBe(true);
      expect(
        samples.every(frame => frame.percentages.length === 1 && frame.percentages[0] === '80%'),
      ).toBe(true);
      expect(samples.every(frame => frame.numbers.length === 1 && frame.numbers[0] === '80')).toBe(
        true,
      );
      expect(state.requests).toHaveLength(before);
      expect(state.documents).toHaveLength(documents);
      expect(state.rsc).toEqual([]);
      expect(await page.evaluate(() => performance.timeOrigin)).toBe(timeOrigin);
      await test.info().attach('cached-table-frames', {
        body: JSON.stringify(samples),
        contentType: 'application/json',
      });

      // Returning to Overview remounts the tables, but must not replay from zero.
      const traffic = page.getByRole('navigation', { name: 'Traffic reports' });
      await traffic.getByRole('link', { name: 'Events', exact: true }).click();
      await expect(traffic.getByRole('link', { name: 'Events', exact: true })).toHaveAttribute(
        'aria-current',
        'page',
      );
      await expect(page.getByRole('tab', { name: 'Chart', exact: true })).toBeVisible();
      await recordFrames(page, 'main');
      await traffic.getByRole('link', { name: 'Overview', exact: true }).click();
      await expect(page.getByRole('tab', { name: 'Path', exact: true })).toBeVisible();
      const returnFrames = await frames(page);
      expect(returnFrames.some(frame => frame.numbers.length > 0)).toBe(true);
      expect(returnFrames.flatMap(frame => frame.numbers).every(value => value === '80')).toBe(
        true,
      );
    });

    test(`native code and data loading stay structured and progressive: ${width} ${reducedMotion}`, async ({
      page,
    }) => {
      test.setTimeout(60_000);
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion });
      const state = await fixture(page, 'app.ios');
      await expect(page.getByRole('heading', { name: 'Daily Active Users' })).toBeVisible();
      const header = await page.getByRole('main').locator('header').first().elementHandle();
      const timeOrigin = await page.evaluate(() => performance.timeOrigin);
      const documents = state.documents.length;
      const navigation = page.getByRole('navigation', { name: 'App reports', exact: true });
      const releaseCode = state.hold('code');
      const releaseData = state.hold('acquisition');
      try {
        await navigation.getByRole('link', { name: 'Customers', exact: true }).click();
        const skeleton = page.locator('[data-report-skeleton="acquisition"]');
        await expect(skeleton).toBeVisible();
        expect(state.requests).not.toContain('acquisition');
        const codeBox = await skeleton.boundingBox();
        await recordFrames(page, 'main');
        releaseCode();
        await expect.poll(() => state.requests.includes('acquisition')).toBe(true);
        await expect(skeleton).toBeVisible();
        expect(await skeleton.boundingBox()).toEqual(codeBox);
        await page.screenshot({ path: test.info().outputPath('acquisition-loading.png') });
        expect((await frames(page)).every(frame => frame.skeletons > 0)).toBe(true);
      } finally {
        releaseCode();
        releaseData();
      }
      await expect(
        page.getByRole('heading', { name: 'Hourly New Users', exact: true }),
      ).toBeVisible();
      const releaseDevices = state.hold('devices');
      try {
        await navigation.getByRole('link', { name: 'Metrics', exact: true }).click();
        await expect.poll(() => state.requests.includes('devices')).toBe(true);
        await expect(page.locator('[data-report-skeleton="table"]')).toHaveCount(8);
        await expect(page.getByRole('heading', { name: 'Models', exact: true })).toBeVisible();
        await expect(page.locator('[data-slot="loading"]')).toHaveCount(0);
        await page.screenshot({ path: test.info().outputPath('devices-loading.png') });
      } finally {
        releaseDevices();
      }
      await expect(page.getByTitle('80')).toHaveCount(8);
      const sidebar = page.getByRole('navigation', { name: 'Metrics reports' });
      const releaseSdk = state.hold('sdkVersion');
      try {
        await sidebar.getByRole('link', { name: 'Versions', exact: true }).click();
        await expect(page.getByText('appVersion-ready', { exact: true })).toBeVisible();
        await expect(page.getByText('buildNumber-ready', { exact: true })).toBeVisible();
        await expect(page.locator('[data-report-skeleton="versions"]')).toHaveCount(1);
        await page.screenshot({ path: test.info().outputPath('versions-progressive.png') });
      } finally {
        releaseSdk();
      }
      await expect(page.getByText('sdkVersion-ready', { exact: true })).toBeVisible();
      const before = state.requests.length;
      await recordFrames(page, 'main');
      await sidebar.getByRole('link', { name: 'Devices', exact: true }).click();
      await expect(page.getByTitle('80')).toHaveCount(8);
      const samples = await frames(page);
      expect(samples.some(frame => frame.numbers.length === 8)).toBe(true);
      expect(samples.flatMap(frame => frame.numbers).every(value => value === '80')).toBe(true);
      expect(samples.flatMap(frame => frame.percentages).every(value => value === '80%')).toBe(
        true,
      );
      expect(samples.every(frame => frame.skeletons === 0)).toBe(true);
      expect(state.requests).toHaveLength(before);
      expect(await header?.evaluate(node => node.isConnected)).toBe(true);
      expect(state.documents).toHaveLength(documents);
      expect(state.rsc).toEqual([]);
      expect(await page.evaluate(() => performance.timeOrigin)).toBe(timeOrigin);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await test.info().attach('cached-native-frames', {
        body: JSON.stringify(samples),
        contentType: 'application/json',
      });
    });
  }
}
