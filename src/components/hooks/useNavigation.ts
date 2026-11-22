import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { createContext, startTransition, useCallback, useContext, useMemo } from 'react';
import { buildPath } from '@/lib/url';

export const SpaNavigationContext = createContext<string | null>(null);

export function useNavigationRouter() {
  const nextRouter = useRouter();
  const root = useContext(SpaNavigationContext);
  return useMemo(() => {
    const localUrl = (href: string) => {
      if (!root || typeof window === 'undefined') return null;
      const basePath = process.env.basePath || '';
      const url = new URL(href, window.location.href);
      if (basePath && href.startsWith(root)) url.pathname = basePath + url.pathname;
      const scope = basePath + root;
      return url.origin === window.location.origin &&
        (url.pathname === scope || url.pathname.startsWith(`${scope}/`))
        ? url
        : null;
    };
    const navigate =
      (method: 'push' | 'replace'): typeof nextRouter.push =>
      (href, options) => {
        const url = localUrl(href);
        if (!url) return nextRouter[method](href, options);
        // Rust serves this client-dispatched shell, not Next's RSC navigation endpoint.
        startTransition(() => window.history[`${method}State`](null, '', url));
      };
    return {
      localUrl,
      router: root
        ? {
            ...nextRouter,
            push: navigate('push'),
            replace: navigate('replace'),
            prefetch: (...args: Parameters<typeof nextRouter.prefetch>) => {
              if (!localUrl(args[0])) nextRouter.prefetch(...args);
            },
          }
        : nextRouter,
    };
  }, [nextRouter, root]);
}

export function useNavigation() {
  const { router } = useNavigationRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const [, teamId] = pathname.match(/\/teams\/([a-f0-9-]+)/) || [];
  const [, websiteId] = pathname.match(/\/websites\/([a-f0-9-]+)/) || [];
  const [, linkId] = pathname.match(/\/links\/([a-f0-9-]+)/) || [];
  const [, pixelId] = pathname.match(/\/pixels\/([a-f0-9-]+)/) || [];
  const [, boardId] = pathname.match(/\/boards\/([a-f0-9-]+)/) || [];
  const searchParamsString = searchParams.toString();
  const queryParams = useMemo(() => Object.fromEntries(searchParams), [searchParamsString]);

  const updateParams = useCallback(
    (params?: Record<string, string | number | undefined>) => {
      return buildPath(pathname, { ...queryParams, ...params });
    },
    [pathname, queryParams],
  );

  const replaceParams = useCallback(
    (params?: Record<string, string | number | undefined>) => {
      return buildPath(pathname, params);
    },
    [pathname],
  );

  const renderUrl = useCallback(
    (path: string, params?: Record<string, string | number | undefined> | false) => {
      return buildPath(
        teamId ? `/teams/${teamId}${path}` : path,
        params === false ? {} : { ...queryParams, ...params },
      );
    },
    [teamId, queryParams],
  );

  return {
    router,
    pathname,
    searchParams,
    query: queryParams,
    teamId,
    websiteId,
    linkId,
    pixelId,
    boardId,
    updateParams,
    replaceParams,
    renderUrl,
  };
}
