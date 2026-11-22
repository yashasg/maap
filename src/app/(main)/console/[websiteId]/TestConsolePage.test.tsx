import { afterEach, expect, test, vi } from 'vitest';
import { render, screen } from '@/test/render';
import { TestConsolePage } from './TestConsolePage';

vi.mock('@/components/hooks', async importOriginal => ({
  ...(await importOriginal<typeof import('@/components/hooks')>()),
  useWebsiteQuery: () => ({ data: { name: 'Kashiworks' } }),
}));
vi.mock('@/components/common/Link', () => ({
  default: ({ children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) => (
    <a {...props}>{children}</a>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

test('console clicks and remounts never install collectors or send production analytics', async () => {
  const fetch = vi.fn();
  vi.stubGlobal('fetch', fetch);
  const originalTracker = window.umami;
  const pushState = window.history.pushState;
  const replaceState = window.history.replaceState;
  const websiteId = 'aa373682-082b-4b6c-baa8-8736d78d6c8d';
  for (let visit = 0; visit < 2; visit++) {
    const { user, unmount } = render(<TestConsolePage websiteId={websiteId} />);
    expect(screen.getByLabelText('Local events')).toHaveTextContent('Run a script');
    await user.click(screen.getByRole('button', { name: 'Send event' }));
    expect(screen.getByRole('heading', { name: 'Local events (1)' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Run script' }));
    expect(screen.getByRole('heading', { name: 'Local events (4)' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Run identify' }));
    await user.click(screen.getByRole('button', { name: 'Revenue script' }));
    expect(
      document.querySelector('script[src*="script.js"], script[src*="recorder.js"]'),
    ).toBeNull();
    unmount();
    window.history.pushState({}, '', '/websites');
    window.history.replaceState({}, '', '/');
    document.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(window.umami).toBe(originalTracker);
    expect(window.history.pushState).toBe(pushState);
    expect(window.history.replaceState).toBe(replaceState);
  }
  expect(fetch).not.toHaveBeenCalled();
});
