import { useConfig } from '@/components/hooks';
import { FAVICON_URL, GROUPED_DOMAINS } from '@/lib/constants';

const PLATFORM_ICONS: Record<string, string> = {
  'app.ios': '/images/os/ios.png',
  'app.macos': '/images/os/mac-os.png',
  'app.android': '/images/os/android-os.png',
};

function getHostName(url: string) {
  const match = url.match(/^(?:https?:\/\/)?(?:[^@\n]+@)?([^:/\n?=]+)/im);
  return match && match.length > 1 ? match[1] : null;
}

export function Favicon({ domain, ...props }) {
  const config = useConfig();

  if (domain && /(?:apps|itunes)\.apple\.com\/|play\.google\.com\//i.test(domain)) {
    return (
      <img
        src={`/app-icon?url=${encodeURIComponent(domain)}`}
        width={16}
        height={16}
        alt=""
        style={{ borderRadius: 4, ...props?.style }}
        {...props}
      />
    );
  }

  const hostName = domain ? getHostName(domain) : null;
  const platformIcon = hostName ? PLATFORM_ICONS[hostName] : undefined;

  if (platformIcon) {
    return <img src={platformIcon} width={16} height={16} alt="" {...props} />;
  }

  if (config?.privateMode) {
    return null;
  }

  const url = config?.faviconUrl || FAVICON_URL;
  const domainName = GROUPED_DOMAINS[hostName]?.domain || hostName;
  const src = hostName ? url.replace(/\{\{\s*domain\s*}}/, domainName) : null;

  return hostName ? <img src={src} width={16} height={16} alt="" {...props} /> : null;
}
