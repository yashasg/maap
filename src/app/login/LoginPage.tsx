'use client';
import { Column, Loading } from '@umami/react-zen';
import { usePathname, useRouter } from 'next/navigation';
import { type PropsWithChildren, useEffect } from 'react';
import { useLoginQuery } from '@/components/hooks';
import { LogoutPage } from '../logout/LogoutPage';
import { LoginForm } from './LoginForm';

export function LoginPageWrapper({ children }: PropsWithChildren) {
  const { user, isLoading } = useLoginQuery();
  const router = useRouter();

  useEffect(() => {
    if (user) {
      router.replace('/');
    }
  }, [user, router]);

  if (isLoading || user) {
    return <Loading placement="absolute" />;
  }

  return (
    <Column
      alignItems="center"
      justifyContent="flex-start"
      height="100vh"
      backgroundColor="surface-raised"
      style={{ paddingTop: '15vh' }}
    >
      {children}
    </Column>
  );
}

export function LoginPage() {
  // The standalone server uses this static shell for the otherwise dynamic logout page.
  if (usePathname() === '/logout') return <LogoutPage />;
  return (
    <LoginPageWrapper>
      <LoginForm />
    </LoginPageWrapper>
  );
}
