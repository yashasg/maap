import type { Metadata } from 'next';
import { SpaRouteView } from '../SpaRouteView';
import { WebsitesPage } from './WebsitesPage';

export default function () {
  return (
    <SpaRouteView>
      <WebsitesPage />
    </SpaRouteView>
  );
}

export const metadata: Metadata = {
  title: 'Websites',
};
