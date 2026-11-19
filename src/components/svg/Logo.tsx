import type { SVGProps } from 'react';

const SvgLogo = (props: SVGProps<SVGSVGElement>) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width={24}
    height={24}
    viewBox="0 0 256 256"
    fill="none"
    {...props}
  >
    <image href="/logo.png" width="256" height="256" preserveAspectRatio="xMidYMid meet" />
  </svg>
);
export default SvgLogo;
