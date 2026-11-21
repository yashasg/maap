import { notFound } from 'next/navigation';
import { AppReportPage } from '../../app-metrics/AppReportPage';
import { isAppReport } from '../../app-metrics/sections';

export default async function Page({
  params,
}: {
  params: Promise<{ websiteId: string; section: string }>;
}) {
  const { websiteId, section } = await params;
  if (!isAppReport(section)) notFound();
  return <AppReportPage websiteId={websiteId} screen={section} />;
}
