import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, test, vi } from 'vitest';
import { setTestUrl } from '@/test/navigation';
import { render } from '@/test/test-utils';
import SetupScreen from './Screen';

const mockUseWebsiteQuery = vi.fn();

vi.mock('@/components/hooks/queries/useWebsiteQuery', () => ({
  useWebsiteQuery: (...args: unknown[]) => mockUseWebsiteQuery(...args),
}));

const websiteId = '3d452373-9d34-41f4-a80d-5b03add59b1c';

beforeEach(() => {
  setTestUrl(
    `/websites/${websiteId}/app-reports/setup?startAt=1&endAt=2&timezone=America%2FLos_Angeles`,
  );
  mockUseWebsiteQuery.mockReturnValue({
    data: { id: websiteId, name: 'Pocket Library' },
    isLoading: false,
    error: null,
  });
});

test('shows app-specific verified Swift setup and preserves validation context', async () => {
  render(<SetupScreen websiteId={websiteId} />);

  await screen.findByText(websiteId);

  expect(screen.getByText(`${window.location.origin}/`)).toBeInTheDocument();
  const namespaceBody = screen.getByText(/Maap uses the App ID to select the app/);
  expect(namespaceBody.closest('[data-slot="alert-description"]')).not.toBeNull();
  expect(screen.getByText('https://github.com/TelemetryDeck/SwiftSDK.git')).toBeInTheDocument();
  expect(screen.getByText(/Exact Version 2\.14\.2/)).toBeInTheDocument();

  const validationLink = screen.getByRole('button', {
    name: 'Open Recent Events in Test Mode',
  });
  expect(validationLink).toHaveAttribute(
    'href',
    `/websites/${websiteId}/app-reports/recent-events?startAt=1&endAt=2&timezone=America%2FLos_Angeles&testMode=true`,
  );

  const notice = screen.getByText(/Only Swift .* is verified with Maap/);
  expect(notice).not.toBeVisible();
  fireEvent.click(screen.getByText('Other SDKs'));
  expect(notice).toBeVisible();
  expect(screen.getByText('https://github.com/TelemetryDeck/SwiftSDK.git')).toBeVisible();

  fireEvent.click(screen.getByRole('button', { name: 'Copy app ID' }));
  await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(websiteId));
});
