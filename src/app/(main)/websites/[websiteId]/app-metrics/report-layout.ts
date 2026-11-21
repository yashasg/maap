import { detectPlatform } from '../../WebsiteAddForm';
import { isAppReport } from './sections';

export function isNativeApp(domain?: string | null) {
  return !!domain && detectPlatform(domain) !== 'website';
}

export function isPropertyPath(pathname: string, websiteId?: string) {
  if (!websiteId) return false;
  const path = pathname.replace(/^\/teams\/[a-f0-9-]+(?=\/)/i, '').replace(/\/+$/, '');
  const root = `/websites/${websiteId}`;
  return path === root || path.startsWith(`${root}/`);
}

export function isAppReportPath(pathname: string, websiteId?: string) {
  if (!websiteId) return false;
  const path = pathname.replace(/^\/teams\/[a-f0-9-]+(?=\/)/i, '').replace(/\/+$/, '');
  const root = `/websites/${websiteId}`;
  const prefix = `${root}/app-reports/`;
  return path === root || (path.startsWith(prefix) && isAppReport(path.slice(prefix.length)));
}
