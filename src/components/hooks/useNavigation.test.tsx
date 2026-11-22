import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { expect, test, vi } from 'vitest';
import { getTestRouter, setTestUrl } from '@/test/navigation';
import { SpaNavigationContext, useNavigation } from './useNavigation';

const root = '/teams/team/websites/website';
const wrapper = ({ children }: { children: ReactNode }) => (
  <SpaNavigationContext.Provider value={root}>{children}</SpaNavigationContext.Provider>
);

test('static-shell report links and query changes use history, not RSC navigation', () => {
  setTestUrl(`${root}/app-reports/recent-events?testMode=true`);
  const push = vi.spyOn(window.history, 'pushState');
  const replace = vi.spyOn(window.history, 'replaceState');
  const { result } = renderHook(useNavigation, { wrapper });
  act(() => result.current.router.push(result.current.updateParams({ page: 2 })));
  expect(window.location.search).toBe('?testMode=true&page=2');
  expect(push).toHaveBeenCalledOnce();
  act(() => result.current.router.replace(`${root}/app-reports/parameters?date=30day`));
  expect(window.location.pathname).toBe(`${root}/app-reports/parameters`);
  expect(replace).toHaveBeenCalledOnce();
  result.current.router.prefetch(`${root}/app-reports/devices`);
  expect(getTestRouter().push).not.toHaveBeenCalled();
  expect(getTestRouter().replace).not.toHaveBeenCalled();
  expect(getTestRouter().prefetch).not.toHaveBeenCalled();
});

test('leaves Next-rendered pages and navigation outside the current shell alone', () => {
  setTestUrl(`${root}/events`);
  const { result } = renderHook(useNavigation, { wrapper });
  for (const href of ['/settings', `${root}-other/events`, `https://example.com${root}/events`]) {
    result.current.router.push(href, { scroll: false });
    expect(getTestRouter().push).toHaveBeenLastCalledWith(href, { scroll: false });
  }
  const nextPage = renderHook(useNavigation);
  expect(nextPage.result.current.router).toBe(getTestRouter());
});

test('preserves the configured base path for local navigation', () => {
  vi.stubEnv('basePath', '/analytics');
  try {
    setTestUrl(`/analytics${root}/events`);
    const { result } = renderHook(useNavigation, { wrapper });
    act(() => result.current.router.push(`${root}/sessions?page=2`));
    expect(window.location.pathname).toBe(`/analytics${root}/sessions`);
    expect(window.location.search).toBe('?page=2');
    act(() => result.current.router.replace(`/analytics${root}/events`));
    expect(window.location.pathname).toBe(`/analytics${root}/events`);
    expect(getTestRouter().push).not.toHaveBeenCalled();
  } finally {
    vi.unstubAllEnvs();
  }
});
