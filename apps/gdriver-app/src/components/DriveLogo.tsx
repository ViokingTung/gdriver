export default function DriveLogo({ size = 48 }: { size?: number }) {
  const aspect = 84 / 71;
  const w = size;
  const h = size / aspect;

  return (
    <svg
      width={w}
      height={h}
      viewBox="0 0 84 71"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-label="Google Drive"
    >
      <path
        d="M55.9721 46.9762L41.9989 70.9995L28.0256 46.9762H55.9721Z"
        fill="#FBBC05"
      />
      <path
        d="M70.0003 23.5838L55.9998 47.0002L41.9993 23.5838H70.0003Z"
        fill="#34A853"
      />
      <path
        d="M55.973 46.9763L41.9991 70.9997L14.0281 23.023H41.9991L55.973 46.9763Z"
        fill="#4285F4"
      />
      <path
        d="M41.9998 23.0234L27.9996 46.4398L14 23.0234L27.9998 -0.000244141L41.9998 23.0234Z"
        fill="#EA4335"
      />
      <path
        d="M14.0002 23.0239L28.0004 -0.000366211L42.0006 23.0239L28.0004 46.4402L14.0002 23.0239Z"
        fill="#1967D2"
      />
    </svg>
  );
}
