'use client';
import { Button, Column, Grid, Heading } from '@umami/react-zen';
import { type MouseEvent, useState } from 'react';
import Link from '@/components/common/Link';
import { PageBody } from '@/components/common/PageBody';
import { PageHeader } from '@/components/common/PageHeader';
import { Panel } from '@/components/common/Panel';
import { useWebsiteQuery } from '@/components/hooks';
import type { CustomEventFunction, EventData, PageViewProperties } from '@/tracker';

export function TestConsolePage({ websiteId }: { websiteId: string }) {
  const { data } = useWebsiteQuery(websiteId);
  const [events, setEvents] = useState<{ type: string; payload: unknown }[]>([]);

  function collect(type: string, payload: unknown) {
    // ponytail: a bounded, in-memory preview; never install a production tracker or recorder.
    setEvents(previous => [...previous.slice(-99), { type, payload }]);
  }

  function track(name?: string | CustomEventFunction, eventData?: EventData) {
    const payload: PageViewProperties = {
      website: websiteId,
      hostname: window.location.hostname,
      url: window.location.href,
      title: document.title,
    };
    collect(
      'event',
      typeof name === 'function' ? name(payload) : { ...payload, name, data: eventData },
    );
  }

  function handleClick(event: MouseEvent<HTMLDivElement>) {
    if (!(event.target instanceof Element)) return;
    const target = event.target.closest('[data-umami-event]');
    if (target) {
      const eventData = Object.fromEntries(
        target
          .getAttributeNames()
          .filter(name => name.startsWith('data-umami-event-'))
          .map(name => [name.slice('data-umami-event-'.length), target.getAttribute(name)]),
      );
      track(target.getAttribute('data-umami-event'), eventData);
    }
  }

  function handleRunScript() {
    track(props => ({
      ...props,
      url: '/page-view',
      referrer: 'https://www.google.com',
    }));
    track('track-event-no-data');
    track('track-event-with-data', {
      test: 'test-data',
      boolean: true,
      booleanError: 'true',
      time: new Date().toISOString(),
      user: `user${Math.round(Math.random() * 10)}`,
      number: 1,
      number2: Math.random() * 100,
      time2: new Date().toISOString(),
      nested: {
        test: 'test-data',
        number: 1,
        object: {
          test: 'test-data',
        },
      },
      array: [1, 2, 3],
    });
  }

  function handleRunRevenue() {
    track(props => ({
      ...props,
      url: '/checkout-cart',
      referrer: 'https://www.google.com',
    }));
    track('checkout-cart', {
      revenue: parseFloat((Math.random() * 1000).toFixed(2)),
      currency: 'USD',
    });
    track('affiliate-link', {
      revenue: parseFloat((Math.random() * 1000).toFixed(2)),
      currency: 'USD',
    });
    track('promotion-link', {
      revenue: parseFloat((Math.random() * 1000).toFixed(2)),
      currency: 'USD',
    });
    track('checkout-cart', {
      revenue: parseFloat((Math.random() * 1000).toFixed(2)),
      currency: 'EUR',
    });
    track('promotion-link', {
      revenue: parseFloat((Math.random() * 1000).toFixed(2)),
      currency: 'EUR',
    });
    track('affiliate-link', {
      item1: {
        productIdentity: 'ABC424',
        revenue: parseFloat((Math.random() * 10000).toFixed(2)),
        currency: 'JPY',
      },
      item2: {
        productIdentity: 'ZYW684',
        revenue: parseFloat((Math.random() * 10000).toFixed(2)),
        currency: 'JPY',
      },
    });
  }

  function handleRunIdentify() {
    collect('identify', {
      userId: 123,
      name: 'brian',
      number: Math.random() * 100,
      test: 'test-data',
      boolean: true,
      booleanError: 'true',
      time: new Date().toISOString(),
      time2: new Date().toISOString(),
      nested: {
        test: 'test-data',
        number: 1,
        object: {
          test: 'test-data',
        },
      },
      array: [1, 2, 3],
    });
  }

  if (!data) {
    return null;
  }

  return (
    <PageBody>
      <PageHeader title="Test console">
        <Column>{data.name}</Column>
      </PageHeader>
      <Column gap="6" paddingY="6">
        <p>Local preview only. Events are not sent or saved, and recording is disabled.</p>
        <Panel>
          <div onClickCapture={handleClick}>
            <Grid columns="1fr 1fr 1fr" gap>
              <Column gap>
                <Heading>Page links</Heading>
                <div>
                  <Link
                    href={`/console/${websiteId}?page=1`}
                    onClick={() => track(props => ({ ...props, url: '?page=1' }))}
                  >
                    page one
                  </Link>
                </div>
                <div>
                  <Link
                    href={`/console/${websiteId}?page=2`}
                    onClick={() => track(props => ({ ...props, url: '?page=2' }))}
                  >
                    page two
                  </Link>
                </div>
                <div>
                  <a href="https://www.google.com" data-umami-event="external-link-direct">
                    external link (direct)
                  </a>
                </div>
                <div>
                  <a
                    href="https://www.google.com"
                    data-umami-event="external-link-tab"
                    target="_blank"
                    rel="noreferrer"
                  >
                    external link (tab)
                  </a>
                </div>
              </Column>
              <Column gap>
                <Heading>Click events</Heading>
                <Button id="send-event-button" data-umami-event="button-click" variant="primary">
                  Send event
                </Button>
                <Button
                  id="send-event-data-button"
                  data-umami-event="button-click"
                  data-umami-event-name="bob"
                  data-umami-event-id="123"
                  variant="primary"
                >
                  Send event with data
                </Button>
                <Button
                  id="generate-revenue-button"
                  data-umami-event="checkout-cart"
                  data-umami-event-revenue={(Math.random() * 10000).toFixed(2).toString()}
                  data-umami-event-currency="USD"
                  variant="primary"
                >
                  Generate revenue data
                </Button>
                <Button
                  id="button-with-div-button"
                  data-umami-event="button-click"
                  data-umami-event-name={'bob'}
                  data-umami-event-id="123"
                  variant="primary"
                >
                  <div>Button with div</div>
                </Button>
                <div data-umami-event="div-click">DIV with attribute</div>
                <div data-umami-event="div-click-one">
                  <div data-umami-event="div-click-two">
                    <div data-umami-event="div-click-three">Nested DIV</div>
                  </div>
                </div>
              </Column>
              <Column gap>
                <Heading>Javascript events</Heading>
                <Button id="manual-button" variant="primary" onClick={handleRunScript}>
                  Run script
                </Button>
                <Button id="manual-button" variant="primary" onClick={handleRunIdentify}>
                  Run identify
                </Button>
                <Button id="manual-button" variant="primary" onClick={handleRunRevenue}>
                  Revenue script
                </Button>
              </Column>
            </Grid>
          </div>
        </Panel>
        <Heading>Local events ({events.length})</Heading>
        <Panel>
          <pre aria-label="Local events" style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
            {events.length
              ? JSON.stringify(events, null, 2)
              : 'Run a script or click an example to preview its payload.'}
          </pre>
        </Panel>
      </Column>
    </PageBody>
  );
}
