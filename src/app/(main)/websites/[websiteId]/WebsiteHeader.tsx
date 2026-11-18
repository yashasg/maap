import { Icon, Row, Switch, Text } from '@umami/react-zen';
import { Favicon } from '@/components/common/Favicon';
import { IconLabel } from '@/components/common/IconLabel';
import { LinkButton } from '@/components/common/LinkButton';
import { PageHeader } from '@/components/common/PageHeader';
import { useMessages, useNavigation, useWebsite } from '@/components/hooks';
import { Edit } from '@/components/icons';
import { ActiveUsers } from '@/components/metrics/ActiveUsers';

export function WebsiteHeader({
  showActions,
  allowLink = true,
}: {
  showActions?: boolean;
  allowLink?: boolean;
}) {
  const website = useWebsite();
  const { renderUrl, pathname, router, updateParams, query } = useNavigation();
  const isSettings = pathname.endsWith('/settings');
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
        <ActiveUsers websiteId={website.id} allowLink={allowLink} />

        {showActions && (
          <LinkButton href={renderUrl(`/websites/${website.id}/settings`, false)}>
            <IconLabel icon={<Edit />}>{t(labels.edit)}</IconLabel>
          </LinkButton>
        )}
      </Row>
    </PageHeader>
  );
}
