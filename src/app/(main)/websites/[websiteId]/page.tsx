import type { Metadata } from 'next';
import { PropertyHome } from './app-metrics/AppReportPage';

export default async function ({ params }: { params: Promise<{ websiteId: string }> }) {
  const { websiteId } = await params;

  return <PropertyHome websiteId={websiteId} />;
}

export const metadata: Metadata = {
  title: 'Websites',
};
