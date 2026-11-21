import { expect, test } from 'vitest';
import { setTimezone, useApp } from '@/store/app';
import { createTestQueryClient, render, screen } from '@/test/render';
import { SessionActivity } from './SessionActivity';

test('renders session activity timestamps on the correct dashboard day across local midnight', () => {
  const props = {
    websiteId: 'site',
    sessionId: 'session',
    startDate: new Date('2026-11-18T00:00:00Z'),
    endDate: new Date('2026-11-20T00:00:00Z'),
  };
  const queryClient = createTestQueryClient();
  queryClient.setQueryData(
    ['session:activity', props],
    ['2026-11-19T08:15:00.123456Z', '2026-11-19T09:05:00+01:00', '2026-11-19 07:55:00'].map(
      (createdAt, i) => ({ eventId: String(i), createdAt, eventName: `Action ${i}` }),
    ),
  );
  const previous = useApp.getState().timezone;
  setTimezone('America/Los_Angeles');
  try {
    render(<SessionActivity {...props} />, { queryClient });
    expect(screen.getAllByText(/November 19/)).toHaveLength(1);
    expect(screen.getAllByText(/November 18/)).toHaveLength(1);
    expect(screen.getByText(/12:15:00 AM/i)).toBeInTheDocument();
    expect(screen.getByText(/12:05:00 AM/i)).toBeInTheDocument();
    expect(screen.getByText(/11:55:00 PM/i)).toBeInTheDocument();
  } finally {
    setTimezone(previous);
  }
});
