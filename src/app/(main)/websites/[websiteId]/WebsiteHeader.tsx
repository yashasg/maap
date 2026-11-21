import { Row, Switch } from '@umami/react-zen';
import { Favicon } from '@/components/common/Favicon';
import { IconLabel } from '@/components/common/IconLabel';
import { LinkButton } from '@/components/common/LinkButton';
import { PageHeader } from '@/components/common/PageHeader';
import { useMessages, useNavigation, useWebsite } from '@/components/hooks';
import { Edit } from '@/components/icons';
import { ActiveUsers } from '@/components/metrics/ActiveUsers';
import { detectPlatform } from '../WebsiteAddForm';

export function WebsiteHeader({
  showActions,
  allowLink = true,
}: {
  showActions?: boolean;
  allowLink?: boolean;
}) {
  const website = useWebsite();
  const { renderUrl, pathname, router, updateParams, query } = useNavigation();
  const appReports =
    pathname.includes('/app-reports/') ||
    (pathname.replace(/\/$/, '').endsWith(`/websites/${website.id}`) &&
      !!website.domain &&
      detectPlatform(website.domain) !== 'website');
  const isSettings = !appReports && pathname.endsWith('/settings');
  const isTestMode = query.testMode === 'true' || query.tag === 'test' || query.tag === 'eq.test';

  const { t, labels } = useMessages();

  if (isSettings) {
    return null;
  }

  return (
    <PageHeader
      title={website.name}
      icon={<Favicon domain={website.domain} />}
      titleHref={allowLink ? renderUrl(`/websites/${website.id}`, false) : undefined}
    >
      <Row alignItems="center" gap="6" wrap="wrap">
        {!appReports && (
          <Switch
            isSelected={isTestMode}
            onChange={checked =>
              router.push(
                updateParams({
                  testMode: checked ? 'true' : undefined,
                  tag: query.tag === 'test' || query.tag === 'eq.test' ? undefined : query.tag,
                }),
              )
            }
          >
            Test Mode
          </Switch>
        )}
        {!appReports && <ActiveUsers websiteId={website.id} allowLink={allowLink} />}

        {showActions && (
          <LinkButton
            href={renderUrl(
              `/websites/${website.id}/${appReports ? 'app-reports/settings' : 'settings'}`,
            )}
          >
            <IconLabel icon={<Edit />}>{t(labels.edit)}</IconLabel>
          </LinkButton>
        )}
      </Row>
    </PageHeader>
  );
}
