import NextLink from 'next/link';
import { type ComponentPropsWithoutRef, forwardRef } from 'react';
import { useNavigationRouter } from '@/components/hooks/useNavigation';

export type LinkProps = ComponentPropsWithoutRef<typeof NextLink>;
const cloudMode = !!process.env.cloudMode;

const Link = forwardRef<HTMLAnchorElement, LinkProps>(function Link(
  { prefetch, onNavigate, ...props },
  ref,
) {
  const { router, localUrl } = useNavigationRouter();
  const local = typeof props.href === 'string' && localUrl(props.href);
  return (
    <NextLink
      ref={ref}
      {...props}
      prefetch={local ? false : cloudMode ? (prefetch ?? false) : prefetch}
      onNavigate={event => {
        let cancelled = false;
        onNavigate?.({
          preventDefault() {
            cancelled = true;
            event.preventDefault();
          },
        });
        if (local && !cancelled) {
          event.preventDefault();
          router[props.replace ? 'replace' : 'push'](local.href, { scroll: props.scroll });
        }
      }}
    />
  );
});

export default Link;
